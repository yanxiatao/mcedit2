//! 玩家和容器共用的槽位界面；编辑请求交回所属文件 / 对象的既有事务入口。
use iced::{
    widget::{button, column, row, text, text_input, tooltip},
    Element, Length,
};
use mcedit2_world::{
    inventory::{Context, Layout, Snapshot, Spec, Stack},
    nbt_document::{Document, Operation},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
#[path = "inventory_drag.rs"]
mod drag;
#[derive(Debug, Clone, Copy)]
pub(crate) enum Field {
    Id,
    Count,
    Damage,
    Destination,
    Search,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Select(Arc<Snapshot>, i16),
    Input(Arc<()>, Field, String),
    Apply(Arc<()>),
    Picker(Arc<()>, bool),
    Page(Arc<()>, usize),
    Preset(Arc<()>, usize),
    StagePreset(Arc<()>, usize),
    ResetDraft(Arc<()>),
    Copy(Arc<()>),
    Cut(Arc<()>),
    Delete(Arc<()>),
    Move(Arc<()>),
    MoveMode(Arc<()>),
    CancelMove,
    Paste(Arc<()>, Arc<Stack>),
    Drop(Arc<Snapshot>, i16, crate::drag_drop::Drop),
    Nbt(Arc<()>),
    Clear(Arc<Snapshot>),
    Manual(Arc<Snapshot>),
    Disabled(Arc<()>, bool),
}
pub(crate) enum Command {
    None,
    Copied(Arc<Stack>),
    Nbt(String, u8),
    Edit {
        operations: Vec<Operation>,
        spec: Spec,
        slot: i16,
        copied: Option<Arc<Stack>>,
    },
}
struct Cached {
    revision: Arc<()>,
    spec: Spec,
    result: Result<Arc<Snapshot>, String>,
}
struct Active {
    snapshot: Arc<Snapshot>,
    slot: i16,
    id: String,
    count: String,
    damage: String,
    destination: String,
    search: String,
    page: usize,
    picker: bool,
    preset: Option<Arc<Stack>>,
}
struct Search {
    version: Option<i32>,
    source: Option<crate::textures::VisibilityIdentity>,
    language: crate::language::Language,
    query: String,
    result: Result<Arc<Vec<usize>>, String>,
}
#[derive(Default)]
pub(crate) struct State {
    cache: Mutex<BTreeMap<String, Cached>>,
    active: Option<Active>,
    generation: Arc<()>,
    moving: bool,
    icons: Arc<crate::item_icons::Cache>,
    search_cache: Mutex<Option<Search>>,
    drag: drag::Cache,
}
impl State {
    fn search(
        &self,
        version: Option<i32>,
        bank: Option<&crate::textures::TextureBank>,
        language: crate::language::Language,
        query: &str,
    ) -> Result<Arc<Vec<usize>>, String> {
        let mut cache = self.search_cache.lock().expect("物品搜索缓存锁被毒化");
        let source = bank.map(|bank| bank.visibility_id());
        if let Some(cache) = cache.as_ref().filter(|c| {
            c.version == version && c.source == source && c.language == language && c.query == query
        }) {
            return cache.result.clone();
        }
        let result = crate::item_presets::search(version, bank, language, query).map(Arc::new);
        *cache = Some(Search {
            version,
            source,
            language,
            query: query.into(),
            result: result.clone(),
        });
        result
    }
    pub fn invalidate(&mut self) {
        self.active = None;
        self.generation = Arc::new(());
        self.moving = false;
        self.cache.lock().expect("库存界面缓存锁被毒化").clear();
        self.drag.clear();
    }
    fn capture(&self, doc: &Document, spec: Spec) -> Result<Arc<Snapshot>, String> {
        let mut cache = self.cache.lock().expect("库存界面缓存锁被毒化");
        let revision = doc.revision();
        if let Some(saved) = cache
            .get(&spec.path)
            .filter(|saved| saved.spec == spec && Arc::ptr_eq(&saved.revision, &revision))
        {
            return saved.result.clone();
        }
        let result = Snapshot::capture(doc, spec.clone())
            .map(Arc::new)
            .map_err(|e| e.to_string());
        if let Err(error) = &result {
            log::error!("[inventory] {error}");
        }
        cache.insert(
            spec.path.clone(),
            Cached {
                revision,
                spec,
                result: result.clone(),
            },
        );
        result
    }
    fn select(&mut self, snapshot: Arc<Snapshot>, slot: i16) -> Result<(), String> {
        if !snapshot.spec.layout.contains(slot) {
            return Err("所选槽不属于当前库存".into());
        }
        let item = snapshot.items.get(&slot);
        self.active = Some(Active {
            slot,
            id: item.map_or_else(String::new, |item| item.id.clone()),
            count: item.map_or_else(|| "1".into(), |item| item.count.to_string()),
            damage: item.and_then(|item| item.damage).map_or_else(
                || {
                    if item.is_none() && snapshot.spec.context.version == Some(1343) {
                        "0".into()
                    } else {
                        String::new()
                    }
                },
                |damage| damage.to_string(),
            ),
            destination: String::new(),
            search: String::new(),
            page: 0,
            picker: false,
            preset: None,
            snapshot,
        });
        self.generation = Arc::new(());
        self.moving = false;
        Ok(())
    }
    pub fn did_edit(&mut self, doc: &Document, spec: Spec, slot: i16) -> Result<(), String> {
        let snapshot = self.capture(doc, spec)?;
        self.select(snapshot, slot)
    }
    fn active(&self, doc: &Document, generation: &Arc<()>) -> Result<&Active, String> {
        if !Arc::ptr_eq(generation, &self.generation) {
            return Err("所选物品已改变，请使用当前槽位".into());
        }
        let active = self.active.as_ref().ok_or("请先选择槽位")?;
        active.snapshot.verify(doc).map_err(|e| e.to_string())?;
        Ok(active)
    }
    pub fn update(
        &mut self,
        doc: &Document,
        action: Action,
        editable: bool,
    ) -> Result<Command, String> {
        let mutation = matches!(
            action,
            Action::Apply(_)
                | Action::Cut(_)
                | Action::Delete(_)
                | Action::Move(_)
                | Action::MoveMode(_)
                | Action::Paste(..)
                | Action::Drop(..)
                | Action::Clear(_)
                | Action::Manual(_)
                | Action::Disabled(..)
        ) || self.moving && matches!(action, Action::Select(..));
        if mutation && !editable {
            return Err("当前文档为只读、正在处理任务或有待确认候选".into());
        }
        if let Action::Drop(snapshot, slot, drop) = action {
            drop.proof.claim()?;
            snapshot.verify(doc).map_err(|error| error.to_string())?;
            let crate::drag_drop::Payload::Item(stack) = drop.payload else {
                return Err("库存槽位只接收完整物品".into());
            };
            let operations = snapshot
                .paste(slot, &stack)
                .map_err(|error| error.to_string())?;
            if operations.is_empty() {
                return Ok(Command::None);
            }
            return Ok(Command::Edit {
                operations,
                spec: snapshot.spec.clone(),
                slot,
                copied: None,
            });
        }
        if let Action::Select(snapshot, slot) = action {
            snapshot.verify(doc).map_err(|e| e.to_string())?;
            if self.moving {
                let active = self.active(doc, &self.generation)?;
                let operations = active
                    .snapshot
                    .transfer_to(&snapshot, active.slot, slot)
                    .map_err(|e| e.to_string())?;
                return Ok(Command::Edit {
                    operations,
                    spec: snapshot.spec.clone(),
                    slot,
                    copied: None,
                });
            }
            self.select(snapshot, slot)?;
            return Ok(Command::None);
        }
        if let Action::Manual(snapshot) = action {
            snapshot.verify(doc).map_err(|e| e.to_string())?;
            let operations = snapshot.make_manual().map_err(|e| e.to_string())?;
            return Ok(Command::Edit {
                operations,
                spec: snapshot.spec.clone(),
                slot: 0,
                copied: None,
            });
        }
        if let Action::Clear(snapshot) = action {
            snapshot.verify(doc).map_err(|e| e.to_string())?;
            let operations = snapshot.clear().map_err(|e| e.to_string())?;
            return Ok(Command::Edit {
                operations,
                spec: snapshot.spec.clone(),
                slot: 0,
                copied: None,
            });
        }
        if matches!(action, Action::CancelMove) {
            self.moving = false;
            return Ok(Command::None);
        }
        let generation = match &action {
            Action::Input(g, ..)
            | Action::Apply(g)
            | Action::Copy(g)
            | Action::Cut(g)
            | Action::Delete(g)
            | Action::Move(g)
            | Action::MoveMode(g)
            | Action::Paste(g, _)
            | Action::Nbt(g)
            | Action::Picker(g, _)
            | Action::Page(g, _)
            | Action::Preset(g, _)
            | Action::StagePreset(g, _)
            | Action::ResetDraft(g)
            | Action::Disabled(g, _) => Arc::clone(g),
            _ => unreachable!("已处理无需选中物品的命令"),
        };
        let active = self.active(doc, &generation)?;
        let keep_picker = matches!(action, Action::StagePreset(..));
        match action {
            Action::Input(_, field, value) => {
                let active = self.active.as_mut().expect("已核对物品");
                match field {
                    Field::Id => active.id = value,
                    Field::Count => active.count = value,
                    Field::Damage => active.damage = value,
                    Field::Destination => active.destination = value,
                    Field::Search => {
                        active.search = value;
                        active.page = 0;
                    }
                };
                Ok(Command::None)
            }
            Action::Picker(_, open) => {
                self.active.as_mut().expect("已核对物品").picker = open;
                Ok(Command::None)
            }
            Action::Page(_, page) => {
                self.active.as_mut().expect("已核对物品").page = page;
                Ok(Command::None)
            }
            Action::Preset(_, index) | Action::StagePreset(_, index) => {
                let context = active.snapshot.spec.context.clone();
                let catalog = crate::item_presets::get(context.version)
                    .ok_or("该数据版本尚无完整物品预设")?;
                let preset = catalog.presets.get(index).ok_or("物品预设索引无效")?;
                let stack = Arc::new(preset.stack(context)?);
                let damage = crate::item_icons::number(crate::item_icons::field(
                    Some(&stack.value),
                    "Damage",
                ));
                let active = self.active.as_mut().expect("已核对物品");
                active.id = preset.id.clone();
                active.count = preset.count.to_string();
                active.damage = damage.map_or_else(String::new, |d| d.to_string());
                active.preset = Some(stack);
                active.picker = keep_picker;
                self.moving = false;
                Ok(Command::None)
            }
            Action::ResetDraft(_) => {
                let snapshot = Arc::clone(&active.snapshot);
                let slot = active.slot;
                self.select(snapshot, slot)?;
                Ok(Command::None)
            }
            Action::Copy(_) => Ok(Command::Copied(Arc::new(
                active
                    .snapshot
                    .copy(active.slot)
                    .map_err(|e| e.to_string())?,
            ))),
            Action::Nbt(_) => Ok(if active.snapshot.items.contains_key(&active.slot) {
                Command::Nbt(
                    active
                        .snapshot
                        .item_path(active.slot)
                        .map_err(|e| e.to_string())?,
                    10,
                )
            } else {
                let path = &active.snapshot.spec.path;
                let (parent, _) = path.rsplit_once('/').expect("库存路径已解析");
                let names =
                    mcedit2_world::nbt_document::parse_path(path).map_err(|e| e.to_string())?;
                let name = names.last().expect("库存路径有字段名");
                if let Some(value) = doc.inspect_field(parent, name).map_err(|e| e.to_string())? {
                    Command::Nbt(path.clone(), value.kind())
                } else {
                    Command::Nbt(parent.into(), 10)
                }
            }),
            Action::MoveMode(_) => {
                if !active.snapshot.items.contains_key(&active.slot) {
                    return Err("源槽为空".into());
                }
                self.moving = true;
                Ok(Command::None)
            }
            command => {
                let mut slot = active.slot;
                let mut copied = None;
                let operations = match command {
                    Action::Apply(_) => {
                        let count = active.count.trim().parse().map_err(|_| "数量必须为整数")?;
                        let damage = if active.damage.trim().is_empty() {
                            None
                        } else {
                            Some(
                                active
                                    .damage
                                    .trim()
                                    .parse()
                                    .map_err(|_| "旧版耐久必须为 Short 整数")?,
                            )
                        };
                        if let Some(preset) = &active.preset {
                            preset
                                .with_fields(&active.id, count, damage)
                                .and_then(|stack| active.snapshot.paste(slot, &stack))
                        } else {
                            active.snapshot.edit(slot, &active.id, count, damage)
                        }
                    }
                    Action::Cut(_) => {
                        copied = Some(Arc::new(
                            active.snapshot.copy(slot).map_err(|e| e.to_string())?,
                        ));
                        active.snapshot.remove(slot)
                    }
                    Action::Delete(_) => active.snapshot.remove(slot),
                    Action::Disabled(_, disabled) => active.snapshot.set_disabled(slot, disabled),
                    Action::Move(_) => {
                        slot = active
                            .destination
                            .trim()
                            .parse()
                            .map_err(|_| "目标槽必须为整数")?;
                        active.snapshot.swap(active.slot, slot)
                    }
                    Action::Paste(_, stack) => active.snapshot.paste(slot, &stack),
                    _ => unreachable!("已处理只读物品命令"),
                }
                .map_err(|e| e.to_string())?;
                if operations.is_empty() {
                    return Ok(Command::None);
                }
                Ok(Command::Edit {
                    operations,
                    spec: active.snapshot.spec.clone(),
                    slot,
                    copied,
                })
            }
        }
    }
}
pub(crate) fn spec(
    path: impl Into<String>,
    layout: Layout,
    root: &std::path::Path,
    version: Option<i32>,
) -> Spec {
    Spec {
        path: path.into(),
        layout,
        context: Context {
            root: root.into(),
            version,
        },
    }
}
pub(crate) fn view<'a>(
    state: &'a State,
    doc: &Document,
    spec: Spec,
    title: &str,
    bank: Option<&crate::textures::TextureBank>,
    language: crate::language::Language,
    online_skins: bool,
    clipboard: Option<Arc<Stack>>,
    editable: bool,
    drag_drop: &crate::drag_drop::State,
) -> Element<'a, Action> {
    state.icons.set_online_skins(online_skins);
    let snapshot = match state.capture(doc, spec) {
        Ok(value) => value,
        Err(error) => {
            return column![
                text(title.to_owned()).size(18),
                text(format!("库存读取失败：{error}"))
            ]
            .into()
        }
    };
    let mut body = column![text(title.to_owned()).size(18)].spacing(4);
    let active = state
        .active
        .as_ref()
        .filter(|active| active.snapshot.current(doc) && active.snapshot.spec == snapshot.spec);
    if state.moving {
        body = body.push(
            row![
                text("点击目标槽，移动物品或交换两个槽的内容"),
                button("取消移动").on_press(Action::CancelMove)
            ]
            .spacing(4),
        );
    }
    if let Some(id) = &snapshot.loot_table {
        body = body
            .push(text(format!("该容器使用战利品表：{id}；此处不生成战利品")))
            .push(
                button("改为手动物品并移除战利品表")
                    .on_press_maybe(editable.then(|| Action::Manual(Arc::clone(&snapshot)))),
            );
    }
    let editable = editable && snapshot.loot_table.is_none();
    let (_, height) = crate::inventory_grid::extent(snapshot.spec.layout);
    let grid = crate::inventory_grid::view(snapshot.spec.layout, |slot| {
        let item = snapshot.items.get(&slot);
        let slot_name = match (snapshot.spec.layout, slot) {
            (Layout::Furnace, 0) => "输入".into(),
            (Layout::Furnace, 1) => "燃料".into(),
            (Layout::Furnace, 2) => "产物".into(),
            (Layout::Brewing, 0..=2) => format!("药瓶 {}", slot + 1),
            (Layout::Brewing, 3) => "原料".into(),
            (Layout::Brewing, 4) => "燃料".into(),
            (Layout::EntityItem, _) => "物品".into(),
            (Layout::EntityArmor, 0) => "靴子".into(),
            (Layout::EntityArmor, 1) => "护腿".into(),
            (Layout::EntityArmor, 2) => "胸甲".into(),
            (Layout::EntityArmor, 3) => "头盔".into(),
            (Layout::EntityHands, 0) => "主手".into(),
            (Layout::EntityHands, 1) => "副手".into(),
            (Layout::EntityBody, _) => "身体装备".into(),
            (Layout::EntityAnimalArmor, _) => "动物护甲".into(),
            (Layout::EntityDecor, _) => "动物装饰".into(),
            (Layout::EntitySaddle, _) => "鞍具".into(),
            (Layout::Jukebox, _) => "唱片".into(),
            (Layout::Lectern, _) => "书籍".into(),
            (Layout::Pot, _) => "物品".into(),
            (Layout::Brushable, _) => "藏物".into(),
            (_, -106) => "副手".into(),
            (_, 100) => "靴子".into(),
            (_, 101) => "护腿".into(),
            (_, 102) => "胸甲".into(),
            (_, 103) => "头盔".into(),
            _ => slot.to_string(),
        };
        let id = item.map_or("空", |item| item.id.as_str());
        let name = crate::item_catalog::get(snapshot.spec.context.version)
            .and_then(|c| c.items.get(id))
            .map(|registered| {
                registered.variant_label(
                    bank,
                    language,
                    if snapshot.spec.context.version == Some(1343) {
                        item.and_then(|i| i.damage)
                    } else {
                        None
                    },
                )
            })
            .or_else(|| bank.and_then(|bank| bank.block_name(id, language)))
            .unwrap_or(id);
        let short = name.chars().take(4).collect::<String>();
        let count = if snapshot.disabled.contains(&slot) {
            "禁用".into()
        } else {
            item.map_or(String::new(), |item| item.count.to_string())
        };
        let appearance = crate::item_icons::Appearance::new(
            id,
            snapshot.spec.context.version,
            snapshot.item_value(slot),
        )
        .with_online_skins(online_skins);
        let (icon, diagnostic) = item_image(&state.icons, bank, appearance, item.is_some(), short);
        let tile = button(column![text(slot_name).size(10), icon, text(count).size(11)].spacing(0))
            .padding(3)
            .width(42)
            .height(68)
            .style(if active.is_some_and(|active| active.slot == slot) {
                button::primary
            } else {
                button::secondary
            })
            .on_press(Action::Select(Arc::clone(&snapshot), slot));
        let tile = tooltip(
            tile,
            text(format!("槽 {slot} · {name}\n{id}{diagnostic}")),
            tooltip::Position::Top,
        );
        let tile: Element<'_, Action> = if item.is_some() && !state.moving {
            match state.drag.slot(&snapshot, slot) {
                Ok(stack) => crate::drag_drop::source(
                    drag_drop,
                    crate::drag_drop::Payload::Item(stack),
                    tile.into(),
                    Action::Select(Arc::clone(&snapshot), slot),
                    None,
                ),
                Err(error) => column![tile, text(error).size(10)].into(),
            }
        } else {
            tile.into()
        };
        if editable {
            let snapshot = Arc::clone(&snapshot);
            crate::drag_drop::target(
                drag_drop,
                crate::drag_drop::Kind::Item,
                tile,
                move |drop, _, _| Some(Action::Drop(Arc::clone(&snapshot), slot, drop)),
            )
        } else {
            tile
        }
    });
    body = body.push(
        iced::widget::scrollable(grid)
            .direction(iced::widget::scrollable::Direction::Horizontal(
                iced::widget::scrollable::Scrollbar::new(),
            ))
            .height(height + 12.),
    );
    body = body.push(button("清空全部槽位").on_press_maybe(
        (editable && !snapshot.items.is_empty()).then(|| Action::Clear(Arc::clone(&snapshot))),
    ));
    if let Some(active) = active {
        let g = Arc::clone(&state.generation);
        let occupied = snapshot.items.contains_key(&active.slot);
        let disabled = snapshot.disabled.contains(&active.slot);
        if snapshot.spec.layout == Layout::Crafter {
            let mut toggle = iced::widget::checkbox(disabled).label("禁用此槽");
            if editable && !occupied {
                let token = Arc::clone(&g);
                toggle = toggle
                    .on_toggle(move |disabled| Action::Disabled(Arc::clone(&token), disabled));
            }
            body = body.push(toggle);
        }
        body = body.push(text(format!(
            "槽 {} · {}",
            active.slot,
            if occupied { "已有物品" } else { "空槽" }
        )));
        if let Some(preset) = &active.preset {
            let preview = active
                .count
                .trim()
                .parse::<i32>()
                .map_err(|_| "数量必须为整数".to_owned())
                .and_then(|count| {
                    let damage = if active.damage.trim().is_empty() {
                        Ok(None)
                    } else {
                        active
                            .damage
                            .trim()
                            .parse::<i16>()
                            .map(Some)
                            .map_err(|_| "旧版耐久必须为 Short 整数".to_owned())
                    }?;
                    preset
                        .with_fields(&active.id, count, damage)
                        .map_err(|e| e.to_string())
                });
            let preview: Element<'_, Action> = match preview {
                Ok(stack) => {
                    let appearance = crate::item_icons::Appearance::new(
                        &active.id,
                        snapshot.spec.context.version,
                        Some(&stack.value),
                    )
                    .with_online_skins(online_skins);
                    let (icon, diagnostic) =
                        item_image(&state.icons, bank, appearance, true, active.id.clone());
                    row![
                        icon,
                        text(format!(
                            "已选完整物品预设；应用会替换此槽全部物品数据{diagnostic}"
                        ))
                    ]
                    .spacing(4)
                    .wrap()
                    .into()
                }
                Err(error) => text(format!("预设草稿：{error}")).into(),
            };
            body = body
                .push(preview)
                .push(button("取消预设／还原草稿").on_press(Action::ResetDraft(Arc::clone(&g))));
        }
        body = body.push(
            text_input("物品 ID（旧数字物品填写数字）", &active.id).on_input(input(&g, Field::Id)),
        );
        body = body.push(
            button(if active.picker {
                "关闭物品选择器"
            } else {
                "搜索物品"
            })
            .on_press(Action::Picker(Arc::clone(&g), !active.picker)),
        );
        if active.picker {
            body = body.push(picker(
                state,
                active,
                bank,
                language,
                online_skins,
                &g,
                drag_drop,
            ));
        }
        body = body.push(
            row![
                text_input("数量", &active.count)
                    .width(100)
                    .on_input(input(&g, Field::Count)),
                button("应用物品").on_press_maybe(
                    (editable && !disabled).then_some(Action::Apply(Arc::clone(&g)))
                )
            ]
            .spacing(4)
            .wrap(),
        );
        if !active.damage.is_empty() || snapshot.spec.context.version == Some(1343) {
            body = body.push(
                row![
                    text("旧版耐久"),
                    text_input("填写才设置 Damage", &active.damage)
                        .width(100)
                        .on_input(input(&g, Field::Damage))
                ]
                .spacing(4),
            );
        }
        body = body.push(
            row![
                button("复制").on_press_maybe(occupied.then_some(Action::Copy(Arc::clone(&g)))),
                button("剪切")
                    .on_press_maybe((editable && occupied).then_some(Action::Cut(Arc::clone(&g)))),
                button("粘贴到此槽").on_press_maybe(
                    clipboard
                        .filter(|_| editable && !disabled)
                        .map(|clip| Action::Paste(Arc::clone(&g), clip))
                ),
                button("删除").on_press_maybe(
                    (editable && occupied).then_some(Action::Delete(Arc::clone(&g)))
                ),
                button("完整 NBT").on_press(Action::Nbt(Arc::clone(&g)))
            ]
            .spacing(4)
            .wrap(),
        );
        body = body.push(
            row![
                button("选择移动／交换目标").on_press_maybe(
                    (editable && occupied).then_some(Action::MoveMode(Arc::clone(&g)))
                ),
                text_input("目标槽", &active.destination)
                    .width(85)
                    .on_input(input(&g, Field::Destination)),
                button("移动／交换")
                    .on_press_maybe((editable && occupied).then_some(Action::Move(Arc::clone(&g))))
            ]
            .spacing(4)
            .wrap(),
        );
    }
    body.width(Length::Fill).into()
}

fn item_image<'a>(
    cache: &Arc<crate::item_icons::Cache>,
    bank: Option<&crate::textures::TextureBank>,
    key: crate::item_icons::Appearance,
    occupied: bool,
    short: String,
) -> (Element<'a, Action>, String) {
    if !occupied {
        return (text(short).size(11).height(32).into(), String::new());
    }
    let icon = bank.map(|bank| cache.request(bank, key));
    match icon {
        Some(crate::item_icons::Icon::Ready(handle)) => (
            iced::widget::image(handle).width(32).height(32).into(),
            String::new(),
        ),
        Some(crate::item_icons::Icon::Error(error)) => (
            text("⚠").size(20).height(32).into(),
            format!("\n图标：{error}"),
        ),
        Some(crate::item_icons::Icon::Pending) => {
            (text("…").height(32).into(), "\n正在生成图标".into())
        }
        None => (
            text(short).size(11).height(32).into(),
            "\n尚未加载客户端物品资源".into(),
        ),
    }
}
fn picker<'a>(
    state: &'a State,
    active: &'a Active,
    bank: Option<&crate::textures::TextureBank>,
    language: crate::language::Language,
    online_skins: bool,
    g: &Arc<()>,
    drag_drop: &crate::drag_drop::State,
) -> Element<'a, Action> {
    let version = active.snapshot.spec.context.version;
    let Some(catalog) = crate::item_presets::get(version) else {
        return text("该数据版本尚无准确物品清单，可以直接填写物品 ID").into();
    };
    let ids = match state.search(version, bank, language, &active.search) {
        Ok(ids) => ids,
        Err(error) => return text(format!("物品搜索失败：{error}")).into(),
    };
    const PAGE: usize = 24;
    let pages = ids.len().div_ceil(PAGE).max(1);
    let page = active.page.min(pages - 1);
    let visible = &ids[page * PAGE..ids.len().min((page + 1) * PAGE)];
    let stacks = match state.drag.presets(&active.snapshot.spec.context, visible) {
        Ok(stacks) => stacks,
        Err(error) => return text(format!("物品拖放载荷读取失败：{error}")).into(),
    };
    let mut body =
        column![text_input("按名称、ID 或命名空间搜索", &active.search)
            .on_input(input(g, Field::Search))]
        .spacing(4);
    body = body.push(
        row![
            button("上一页")
                .on_press_maybe((page > 0).then(|| Action::Page(Arc::clone(g), page - 1))),
            text(format!("{} 项 · {}/{} 页", ids.len(), page + 1, pages)),
            button("下一页")
                .on_press_maybe((page + 1 < pages).then(|| Action::Page(Arc::clone(g), page + 1)))
        ]
        .spacing(6),
    );
    body = body
        .push(text("单击预览，双击收起目录；拖到槽位直接放入完整物品，拖放复制物品。").size(12));
    for chunk in visible.chunks(3) {
        let mut line = row![].spacing(3);
        for &index in chunk {
            let preset = &catalog.presets[index];
            let name = match preset.label(bank, language) {
                Ok(v) => v,
                Err(e) => return text(e).into(),
            };
            let details = match preset.details(bank, language) {
                Ok(v) => v,
                Err(e) => return text(e).into(),
            };
            let value = match preset.value() {
                Ok(v) => v,
                Err(e) => return text(format!("预设解码失败：{e}")).into(),
            };
            let appearance = crate::item_icons::Appearance::new(&preset.id, version, Some(value))
                .with_online_skins(online_skins);
            let (icon, diagnostic) = item_image(
                &state.icons,
                bank,
                appearance,
                true,
                name.chars().take(4).collect(),
            );
            // 同名药水、不同附魔等级的书在列表上直接展示差异，而不要求逐一悬停。
            let extra = details
                .lines()
                .skip(1)
                .filter(|line| !line.is_empty())
                .take(2)
                .collect::<Vec<_>>()
                .join(" · ");
            let tile = button(
                row![
                    icon,
                    column![text(name).size(12), text(extra).size(10)].spacing(1)
                ]
                .spacing(4),
            )
            .width(Length::Fill)
            .height(52)
            .style(
                if active
                    .preset
                    .as_ref()
                    .is_some_and(|stack| &stack.value == value)
                {
                    button::primary
                } else {
                    button::secondary
                },
            )
            .on_press(Action::Preset(Arc::clone(g), index));
            let tile = tooltip(
                tile,
                text(format!(
                    "{details}\n{}\n堆叠上限 {} · 耐久上限 {}{diagnostic}",
                    preset.id, preset.max_count, preset.max_damage
                )),
                tooltip::Position::Top,
            );
            line = line.push(crate::drag_drop::source(
                drag_drop,
                crate::drag_drop::Payload::Item(Arc::clone(&stacks[&index])),
                tile.into(),
                Action::StagePreset(Arc::clone(g), index),
                Some(Action::Preset(Arc::clone(g), index)),
            ));
        }
        body = body.push(line);
    }
    if ids.is_empty() {
        body = body.push(text("没有匹配的物品"));
    }
    body.into()
}

fn input(token: &Arc<()>, field: Field) -> impl Fn(String) -> Action {
    let token = Arc::clone(token);
    move |value| Action::Input(Arc::clone(&token), field, value)
}
#[cfg(test)]
impl State {
    pub(crate) fn token(&self) -> Arc<()> {
        Arc::clone(&self.generation)
    }
}
#[cfg(test)]
#[path = "inventory_ui_tests.rs"]
mod tests;
#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run(category);
}
