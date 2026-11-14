//! 独立文件的实际对象表单和物品栏；所有修改交回所属文档的一次历史。
use super::*;
use crate::{block_forms, container_ui, inventory_ui};
use mcedit2_world::{
    inventory::{Layout, Spec, Stack},
    nbt_document::Operation,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Page {
    Properties,
    #[default]
    Items,
    Nbt,
}
#[derive(Default)]
pub(crate) struct State {
    page: Page,
    form: block_forms::State,
    container: container_ui::State,
    inventory: inventory_ui::State,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Page(Page),
    Form(block_forms::Action),
    Container(container_ui::Action),
    Inventory(inventory_ui::Action),
}
#[derive(Debug, Clone)]
pub(crate) struct InventoryEdit {
    pub object: Object,
    pub revision: Arc<()>,
    pub operations: Vec<Operation>,
    pub spec: Spec,
    pub slot: i16,
    pub copied: Option<Arc<Stack>>,
}
#[derive(Debug, Clone)]
pub(crate) struct InventoryReceipt {
    pub spec: Spec,
    pub slot: i16,
    pub copied: Option<Arc<Stack>>,
}
impl State {
    pub(super) fn show_nbt(&mut self) {
        self.page = Page::Nbt;
    }
    pub(super) fn refresh(
        &mut self,
        document: Option<&nbt_document::Document>,
        object: Option<Object>,
        version: Option<i32>,
        same: bool,
    ) {
        self.form = match (document, object) {
            (Some(document), Some(Object::BlockEntity(_))) => {
                block_forms::State::new(document, version)
            }
            _ => Default::default(),
        };
        self.container.invalidate();
        self.inventory.invalidate();
        if !same {
            self.page = if self.form.available() {
                Page::Properties
            } else if let (Some(document), Some(object)) = (document, object) {
                match layouts(document, object, version) {
                    Ok(layouts) if layouts.is_empty() => Page::Nbt,
                    _ => Page::Items,
                }
            } else {
                Page::Nbt
            };
        }
    }
    pub(super) fn received(
        &mut self,
        document: &nbt_document::Document,
        receipt: &InventoryReceipt,
    ) -> Result<(), String> {
        self.inventory
            .did_edit(document, receipt.spec.clone(), receipt.slot)
    }
}
fn layout(
    document: &nbt_document::Document,
    object: Object,
    version: Option<i32>,
) -> Result<Option<Layout>, String> {
    match document
        .inspect_field("", "id")
        .map_err(|e| e.to_string())?
    {
        Some(nbt_document::Value::String(id)) => Ok(match object {
            Object::BlockEntity(_) => mcedit2_world::container_data::layout(&id, version),
            Object::Entity(_) => Layout::entity(&id),
        }),
        Some(_) => Err("对象 id 不是字符串；请在 NBT 页修正".into()),
        None => Ok(None),
    }
}
fn layouts(
    document: &nbt_document::Document,
    object: Object,
    version: Option<i32>,
) -> Result<Vec<Layout>, String> {
    let mut layouts: Vec<_> = layout(document, object, version)?.into_iter().collect();
    if matches!(object, Object::Entity(_)) {
        layouts.extend(
            mcedit2_world::entity_equipment::layouts(document)
                .map_err(|error| error.to_string())?,
        );
    }
    Ok(layouts)
}
fn origin(entry: &Entry) -> Result<([i32; 3], Option<String>), String> {
    let document = entry.document.as_ref().ok_or("结构文件未读取")?;
    let content = document.content();
    let Some(Object::BlockEntity(index)) = entry.object else {
        return Err("当前对象不是方块实体".into());
    };
    let body = content
        .block_entities
        .get(index)
        .ok_or("所选方块实体已移除")?;
    let mut position = [0; 3];
    for axis in 0..3 {
        position[axis] = content.source_min[axis]
            .checked_add(body.position[axis])
            .ok_or("命令方块来源坐标溢出")?;
    }
    Ok((position, content.entity_dimension.clone()))
}
pub(super) fn commands(entry: &Entry) -> Result<Option<crate::command_visuals::Document>, String> {
    if entry.engaged() || entry.object_document.is_none() || !entry.object_ui.form.is_command() {
        return Ok(None);
    }
    let (position, dimension) = origin(entry)?;
    let visuals = entry
        .object_ui
        .form
        .navigation(position, dimension.as_deref())?;
    Ok(Some(crate::command_visuals::Document {
        source: entry.document.as_ref().expect("表单属于当前文件").content(),
        visuals,
    }))
}
enum Command {
    None,
    Edit(Vec<Operation>),
    Inventory(InventoryEdit),
    Copied(Arc<Stack>),
    Goto([f64; 3]),
}
fn handle(
    app: &mut App,
    id: u64,
    object: Object,
    revision: Arc<()>,
    action: Action,
) -> Result<Task<Message>, String> {
    if super::active(app) != Some(id) {
        return Ok(Task::none());
    }
    let entry = app.structures.files.get(&id).ok_or("结构文档已关闭")?;
    if entry.busy
        || entry.engaged()
        || entry.object != Some(object)
        || entry
            .object_document
            .as_ref()
            .is_none_or(|document| !Arc::ptr_eq(&revision, &document.revision()))
    {
        return Ok(Task::none());
    }
    let editable = entry.history_editable();
    let bank = entry
        .scene
        .assets
        .as_ref()
        .map(|assets| Arc::clone(&assets.bank));
    let language = app.language;
    let entry = app.structures.files.get_mut(&id).expect("结构文档存在");
    let document = entry.object_document.as_ref().expect("已核对对象文档");
    let command = match action {
        Action::Page(page) => {
            entry.object_ui.page = page;
            Command::None
        }
        Action::Form(action) => {
            match entry.object_ui.form.update(
                document,
                action,
                editable,
                bank.as_deref(),
                language,
            )? {
                block_forms::Command::None => Command::None,
                block_forms::Command::Edit(operations) => Command::Edit(operations),
                block_forms::Command::Goto(index) => {
                    let (position, dimension) = origin(entry)?;
                    let point = if let Some(index) = index {
                        let visuals = entry
                            .object_ui
                            .form
                            .navigation(position, dimension.as_deref())?;
                        let target = visuals.boxes.get(index).ok_or("该命令目标已不存在")?;
                        std::array::from_fn(|axis| (target.min[axis] + target.max[axis]) * 0.5)
                    } else {
                        position.map(|value| f64::from(value) + 0.5)
                    };
                    let source_min = entry
                        .document
                        .as_ref()
                        .expect("对象属于结构文档")
                        .content()
                        .source_min;
                    Command::Goto(std::array::from_fn(|axis| {
                        point[axis] - f64::from(source_min[axis]) - 0.5
                    }))
                }
            }
        }
        Action::Container(action) => Command::Edit(entry.object_ui.container.update(
            document,
            action,
            editable,
            bank.as_deref(),
            language,
        )?),
        Action::Inventory(action) => match entry
            .object_ui
            .inventory
            .update(document, action, editable)?
        {
            inventory_ui::Command::None => Command::None,
            inventory_ui::Command::Copied(stack) => Command::Copied(stack),
            inventory_ui::Command::Nbt(path, kind) => {
                entry.object_ui.page = Page::Nbt;
                if !path.is_empty() {
                    entry.tree.focus(path, kind, document)?;
                }
                Command::None
            }
            inventory_ui::Command::Edit {
                operations,
                spec,
                slot,
                copied,
            } => Command::Inventory(InventoryEdit {
                object,
                revision,
                operations,
                spec,
                slot,
                copied,
            }),
        },
    };
    match command {
        Command::None => Ok(Task::none()),
        Command::Copied(stack) => {
            app.item_clipboard = Some(stack);
            Ok(Task::none())
        }
        Command::Edit(operations) => {
            if operations.is_empty() {
                return Ok(Task::none());
            }
            super::begin(app, id, Work::EditObject(object, operations))
        }
        Command::Inventory(edit) => {
            if edit.operations.is_empty() {
                return Ok(Task::none());
            }
            super::begin(app, id, Work::Inventory(edit))
        }
        Command::Goto(point) => {
            crate::keyboard_bindings::interrupt(app);
            app.structures
                .files
                .get_mut(&id)
                .expect("文档存在")
                .focus(point);
            Ok(Task::none())
        }
    }
}
pub(crate) fn update(
    app: &mut App,
    id: u64,
    object: Object,
    revision: Arc<()>,
    action: Action,
) -> Task<Message> {
    match handle(app, id, object, revision, action) {
        Ok(task) => task,
        Err(error) => {
            if let Some(entry) = app.structures.files.get_mut(&id) {
                super::report(entry, error);
            }
            Task::none()
        }
    }
}
pub(crate) fn view(app: &App, id: u64) -> Element<'_, Message> {
    let entry = &app.structures.files[&id];
    let (Some(object), Some(source)) = (entry.object, entry.object_document.as_ref()) else {
        return text("请选择对象").into();
    };
    let revision = source.revision();
    let send = move |action| {
        Message::StructureWorkspace(
            id,
            super::Action::Object(object, Arc::clone(&revision), action),
        )
    };
    let state = &entry.object_ui;
    let content = entry
        .document
        .as_ref()
        .expect("对象属于已读取文件")
        .content();
    let bank = entry
        .scene
        .assets
        .as_ref()
        .map(|assets| assets.bank.as_ref());
    let editable = entry.history_editable();
    let mut body = iced::widget::column![row![
        button("属性 / 专用编辑")
            .style(if state.page == Page::Properties {
                button::primary
            } else {
                button::secondary
            })
            .on_press(send(Action::Page(Page::Properties))),
        button("物品 / 装备")
            .style(if state.page == Page::Items {
                button::primary
            } else {
                button::secondary
            })
            .on_press(send(Action::Page(Page::Items))),
        button("完整 NBT")
            .style(if state.page == Page::Nbt {
                button::primary
            } else {
                button::secondary
            })
            .on_press(send(Action::Page(Page::Nbt)))
    ]
    .spacing(5)
    .wrap()]
    .spacing(8);
    let layout = layout(source, object, content.data_version);
    match state.page {
        Page::Properties if state.form.available() => match origin(entry) {
            Ok((position, dimension)) => {
                body = body.push(
                    block_forms::view(
                        &state.form,
                        editable,
                        bank,
                        app.language,
                        position,
                        dimension.as_deref(),
                    )
                    .map({
                        let send = send.clone();
                        move |action| send(Action::Form(action))
                    }),
                )
            }
            Err(error) => body = body.push(text(error)),
        },
        Page::Properties => match layout {
            Ok(Some(_)) if matches!(object, Object::BlockEntity(_)) => {
                body = body.push(
                    container_ui::view(
                        &state.container,
                        source,
                        content.data_version,
                        bank,
                        app.language,
                        editable,
                    )
                    .map({
                        let send = send.clone();
                        move |action| send(Action::Container(action))
                    }),
                )
            }
            Ok(_) => body = body.push(text("当前对象没有专用属性表单；可打开完整 NBT。")),
            Err(error) => body = body.push(text(error)),
        },
        Page::Items => match layouts(source, object, content.data_version) {
            Err(error) => body = body.push(text(error)),
            Ok(layouts) if layouts.is_empty() => {
                body = body.push(text("当前对象没有可编辑的物品栏；可打开完整 NBT。"))
            }
            Ok(layouts) => {
                for layout in layouts {
                    let title = match layout {
                        Layout::EntityItem => "实体物品",
                        Layout::EntityArmor => "护甲",
                        Layout::EntityHands => "双手",
                        Layout::EntityBody => "身体装备",
                        Layout::EntityAnimalArmor => "动物护甲",
                        Layout::EntityDecor => "动物装饰",
                        Layout::EntitySaddle => "鞍具",
                        _ => "容器物品",
                    };
                    body = body.push(
                        inventory_ui::view(
                            &state.inventory,
                            source,
                            inventory_ui::spec(
                                format!("/{}", layout.field()),
                                layout,
                                std::path::Path::new(&content.source_root),
                                content.data_version,
                            ),
                            title,
                            bank,
                            app.language,
                            app.entity_models.online_skins,
                            app.item_clipboard.clone(),
                            editable,
                            &app.drag_drop,
                        )
                        .map({
                            let send = send.clone();
                            move |action| send(Action::Inventory(action))
                        }),
                    );
                }
            }
        },
        Page::Nbt => {
            let revision = source.revision();
            body = body.push(entry.tree.view(source).map(move |action| {
                Message::StructureWorkspace(
                    id,
                    super::Action::Tree(object, Arc::clone(&revision), action),
                )
            }));
        }
    }
    body.into()
}
