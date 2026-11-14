//! 玩家来源列表与独立编辑入口；保留同 UUID 的每个实际文件来源。
use crate::{AdvancedSection, App, Message};
use iced::{
    widget::{button, checkbox, column, row, text, text_input},
    Element, Task,
};
use mcedit2_world::player_catalog::{self, Catalog};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
#[derive(Default)]
pub(crate) struct Panel {
    expanded: bool,
    key: Option<usize>,
    epoch: u64,
    cancel: Arc<AtomicBool>,
    catalog: Option<Arc<Catalog>>,
    page: usize,
    status: String,
    filter: String,
    first: Option<First>,
    profiles: BTreeMap<[u8; 16], String>,
    profile_pending: Option<[u8; 16]>,
}
struct First {
    key: (usize, usize),
    active: Option<u64>,
}
#[derive(Debug, Clone, Copy)]
pub(crate) enum Open {
    Attributes,
    Inventory,
    Ender,
    Move,
    Rotate,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Toggle,
    Scan,
    Scanned(u64, usize, Result<Arc<Catalog>, String>),
    Page(i32),
    Filter(String),
    Open(Target, Open),
    Jump(Target),
    View(Target),
    Profile(Target),
    ProfileLoaded(
        u64,
        usize,
        [u8; 16],
        Result<crate::player_profile::Profile, String>,
    ),
}
/// 按用户看到的实际来源快照绑定操作，列表重读后不能把旧行号解释成另一玩家。
#[derive(Clone, Debug)]
pub(crate) struct Target {
    world: usize,
    catalog: Arc<Catalog>,
    player: usize,
    source: usize,
}
impl Target {
    fn get<'a>(&'a self, app: &App) -> Result<&'a player_catalog::PlayerSource, String> {
        if app.current_key().map(|key| key.0) != Some(self.world)
            || app.structures.active.is_some()
            || app.desktop.show_recent
            || app.player_browser.key != Some(self.world)
            || app
                .player_browser
                .catalog
                .as_ref()
                .is_none_or(|current| !Arc::ptr_eq(current, &self.catalog))
        {
            return Err("玩家来源列表已改变，请从当前列表重新选择".into());
        }
        self.catalog
            .players
            .get(self.player)
            .and_then(|player| player.sources.get(self.source))
            .ok_or_else(|| "请选择实际玩家文件来源".into())
    }
}
impl Panel {
    pub(crate) fn cancel_first(&mut self) {
        self.first = None;
    }
    pub(crate) fn target(&self, player: usize, source: usize) -> Result<Target, String> {
        let catalog = self.catalog.as_ref().ok_or("请先读取玩家来源")?;
        catalog
            .players
            .get(player)
            .and_then(|player| player.sources.get(source))
            .ok_or("玩家来源不存在")?;
        Ok(Target {
            world: self.key.ok_or("玩家列表没有所属世界")?,
            catalog: Arc::clone(catalog),
            player,
            source,
        })
    }
    fn filtered(&self, catalog: &Catalog) -> Vec<usize> {
        let query = self.filter.to_lowercase();
        if query.trim().is_empty() {
            return (0..catalog.players.len()).collect();
        }
        catalog
            .players
            .iter()
            .enumerate()
            .filter_map(|(index, player)| {
                let uuid = player
                    .uuid
                    .map(player_catalog::uuid_string)
                    .unwrap_or_default();
                let haystack = format!(
                    "{} {} {} {} {} {}",
                    player.name.as_deref().unwrap_or(""),
                    uuid,
                    uuid.replace('-', ""),
                    player.dimension.as_deref().unwrap_or(""),
                    player
                        .uuid
                        .and_then(|uuid| self.profiles.get(&uuid))
                        .map_or("", String::as_str),
                    player
                        .sources
                        .iter()
                        .map(
                            |source| match player_catalog::source_location(&source.raw) {
                                Ok(location) =>
                                    format!("{} {}", source.path.display(), location.dimension),
                                Err(error) => format!("{} {error}", source.path.display()),
                            }
                        )
                        .collect::<Vec<_>>()
                        .join(" ")
                )
                .to_lowercase();
                query
                    .split_whitespace()
                    .all(|word| haystack.contains(word))
                    .then_some(index)
            })
            .collect()
    }
    pub(crate) fn invalidate(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.epoch = self.epoch.wrapping_add(1);
        self.key = None;
        self.catalog = None;
        self.status.clear();
        self.page = 0;
        self.expanded = false;
        self.filter.clear();
        self.first = None;
        self.profiles.clear();
        self.profile_pending = None;
    }
    pub(crate) fn view(&self, l: crate::language::Language, online: bool) -> Element<'_, Message> {
        let mut body = column![row![
            button(l.pick(
                if self.expanded {
                    "收起玩家来源"
                } else {
                    "展开玩家来源"
                },
                if self.expanded {
                    "Hide player sources"
                } else {
                    "Show player sources"
                }
            ))
            .on_press(Message::PlayerBrowser(Action::Toggle)),
            button(l.pick("读取玩家列表", "Read player list"))
                .on_press(Message::PlayerBrowser(Action::Scan)),
            button(l.pick("世界属性", "World properties")).on_press(Message::WorldInfo)
        ]
        .spacing(5)
        .wrap()]
        .spacing(5);
        if !self.expanded {
            return body.into();
        }
        body = body.push(text(&self.status)).push(
            checkbox(online)
                .label(l.pick(
                    "在线读取玩家姓名与皮肤",
                    "Fetch player names and skins online",
                ))
                .on_toggle(Message::ShowOnlinePlayerSkins),
        );
        if let Some(catalog) = &self.catalog {
            let filtered = self.filtered(catalog);
            let pages = filtered.len().div_ceil(32).max(1);
            let page = self.page.min(pages - 1);
            body = body
                .push(
                    text_input("按姓名、UUID、维度或来源路径过滤", &self.filter)
                        .on_input(|value| Message::PlayerBrowser(Action::Filter(value))),
                )
                .push(
                    row![
                        button("上一页").on_press_maybe(
                            (page > 0).then_some(Message::PlayerBrowser(Action::Page(-1)))
                        ),
                        text(format!(
                            "匹配 {} / {} 名玩家 · {}/{}",
                            filtered.len(),
                            catalog.players.len(),
                            page + 1,
                            pages
                        )),
                        button("下一页").on_press_maybe(
                            (page + 1 < pages).then_some(Message::PlayerBrowser(Action::Page(1)))
                        ),
                    ]
                    .spacing(5),
                );
            if filtered.is_empty() {
                body = body.push(text("没有匹配的玩家来源"));
            }
            for index in filtered.into_iter().skip(page * 32).take(32) {
                let player = &catalog.players[index];
                let name = player
                    .uuid
                    .and_then(|uuid| self.profiles.get(&uuid).cloned())
                    .or_else(|| player.name.clone())
                    .or_else(|| player.uuid.map(player_catalog::uuid_string))
                    .unwrap_or_else(|| "本地玩家".into());
                body = body.push(text(name.clone()));
                if let Some(uuid) = player.uuid {
                    body = body.push(text(format!("UUID：{}", player_catalog::uuid_string(uuid))));
                    if let Some(saved) = player
                        .name
                        .as_ref()
                        .filter(|saved| saved.as_str() != name.as_str())
                    {
                        body = body.push(text(format!("存档记录姓名：{saved}")));
                    }
                    if !player.sources.is_empty() {
                        let target = self.target(index, 0).expect("玩家行具有实际来源");
                        body = body.push(
                            button(if self.profile_pending == Some(uuid) {
                                "正在读取姓名…"
                            } else if !online {
                                "读取缓存姓名"
                            } else {
                                "读取在线姓名"
                            })
                            .on_press_maybe(
                                self.profile_pending
                                    .is_none()
                                    .then_some(Message::PlayerBrowser(Action::Profile(target))),
                            ),
                        );
                    }
                }
                for (source_index, source) in player.sources.iter().enumerate() {
                    let target = self
                        .target(index, source_index)
                        .expect("显示行来自当前实际来源快照");
                    let location = player_catalog::source_location(&source.raw);
                    body = body.push(text(format!(
                        "{} · {}",
                        source.path.display(),
                        source.nbt_path
                    )));
                    body = body.push(text(match &location {
                        Ok(location) => format!(
                            "读取时：{} · ({}, {}, {})",
                            location.dimension,
                            location.position[0],
                            location.position[1],
                            location.position[2]
                        ),
                        Err(error) => format!("位置不可用：{error}"),
                    }));
                    body =
                        body.push(
                            row![
                                button("前往此来源")
                                    .on_press(Message::PlayerBrowser(Action::Jump(target.clone()))),
                                button(l.pick("玩家视角", "Player view"))
                                    .on_press(Message::PlayerBrowser(Action::View(target.clone()))),
                                button("属性").on_press(Message::PlayerBrowser(Action::Open(
                                    target.clone(),
                                    Open::Attributes
                                ))),
                                button("背包").on_press(Message::PlayerBrowser(Action::Open(
                                    target.clone(),
                                    Open::Inventory
                                ))),
                                button("末影箱").on_press(Message::PlayerBrowser(Action::Open(
                                    target.clone(),
                                    Open::Ender
                                ))),
                                button("移动到镜头").on_press(Message::PlayerBrowser(
                                    Action::Open(target.clone(), Open::Move)
                                )),
                                button("采用镜头朝向").on_press(Message::PlayerBrowser(
                                    Action::Open(target, Open::Rotate)
                                )),
                            ]
                            .spacing(5)
                            .wrap(),
                        );
                }
            }
            for error in &catalog.diagnostics {
                body = body.push(text(error));
            }
        }
        iced::widget::container(
            iced::widget::scrollable(body)
                .height(iced::Length::Shrink)
                .direction(iced::widget::scrollable::Direction::Vertical(
                    iced::widget::scrollable::Scrollbar::new().spacing(8),
                )),
        )
        .max_height(240)
        .width(iced::Length::Fill)
        .into()
    }
}
fn jump(app: &mut App, target: Target, require_rotation: bool) -> Task<Message> {
    let result = (|| -> Result<_, String> {
        let source = target.get(app)?;
        let location = crate::nbt_documents::player_location(app, source)?;
        if require_rotation && location.rotation.is_none() {
            return Err("此玩家来源未保存朝向，只能使用“前往此来源”定位坐标".into());
        }
        Ok((target.world, location))
    })();
    match result {
        Ok((world, location)) => Task::done(Message::ViewState(
            crate::view_state::Action::PlayerSource(world, location),
        )),
        Err(error) => {
            log::error!("[player-browser] {error}");
            app.player_browser.status = error;
            Task::none()
        }
    }
}
fn open_first(app: &mut App) -> Task<Message> {
    let selected = app.player_browser.catalog.as_ref().and_then(|catalog| {
        // 原版优先本地玩家；同 UUID 的多人文件仍保持独立的真实来源。
        catalog
            .players
            .iter()
            .enumerate()
            .find_map(|(player, entry)| {
                entry
                    .sources
                    .iter()
                    .position(|source| source.nbt_path.trim_matches('/') == "Data/Player")
                    .map(|source| (player, source))
            })
            .or_else(|| {
                catalog
                    .players
                    .iter()
                    .enumerate()
                    .find_map(|(player, entry)| (!entry.sources.is_empty()).then_some((player, 0)))
            })
    });
    match selected {
        Some((player, source)) => {
            let target = app
                .player_browser
                .target(player, source)
                .expect("第一玩家来自实际来源清单");
            update(app, Action::Open(target, Open::Attributes))
        }
        None => {
            app.player_browser.expanded = true;
            app.player_browser.status = "当前世界没有可打开的玩家来源".into();
            Task::none()
        }
    }
}
pub(crate) fn show_first(app: &mut App) -> Task<Message> {
    let key = app.current_key().expect("玩家属性入口已核对所属世界");
    if app.player_browser.key == Some(key.0) && app.player_browser.catalog.is_some() {
        return open_first(app);
    }
    let active = app.nbt_documents.active(key.0).map(|(id, _)| id);
    let task = update(app, Action::Scan);
    app.player_browser.first = Some(First { key, active });
    task
}
/// 明确打开某个玩家时仍读取其余实际来源，但不改变已请求的玩家或展开状态。
pub(crate) fn maintain_catalog(app: &mut App) -> Task<Message> {
    let world = app.current_key().expect("玩家页具有所属世界").0;
    if app.player_browser.key == Some(world) {
        return Task::none();
    }
    let expanded = app.player_browser.expanded;
    let task = update(app, Action::Scan);
    app.player_browser.expanded = expanded;
    task
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::Toggle => app.player_browser.expanded = !app.player_browser.expanded,
        Action::Scan => {
            let filter = app.player_browser.filter.clone();
            app.player_browser.invalidate();
            app.player_browser.filter = filter;
            app.player_browser.expanded = true;
            let Some(key) = app.current_key() else {
                app.player_browser.status = "请先选择世界".into();
                return Task::none();
            };
            let world = app.worlds[key.0].clone();
            app.player_browser.key = Some(key.0);
            app.player_browser.cancel = Arc::new(AtomicBool::new(false));
            let cancel = Arc::clone(&app.player_browser.cancel);
            let epoch = app.player_browser.epoch;
            app.player_browser.status = "正在后台读取玩家来源…".into();
            return Task::perform(
                crate::nbt_background(move || {
                    player_catalog::load_all(&world, &|| cancel.load(Ordering::Acquire))
                        .map(Arc::new)
                        .map_err(|error| error.to_string())
                }),
                move |result| Message::PlayerBrowser(Action::Scanned(epoch, key.0, result)),
            );
        }
        Action::Scanned(epoch, key, result) => {
            if epoch != app.player_browser.epoch || Some(key) != app.current_key().map(|key| key.0)
            {
                return Task::none();
            }
            let first = app.player_browser.first.take();
            match result {
                Ok(catalog) => {
                    app.player_browser.status = format!(
                        "读取 {} 名玩家；同身份的文件来源分别列出",
                        catalog.players.len()
                    );
                    app.player_browser.catalog = Some(catalog);
                    if first.is_some_and(|first| {
                        app.current_key() == Some(first.key)
                            && !app.desktop.show_recent
                            && app.structures.active.is_none()
                            && app.show_advanced
                            && app.advanced_section == AdvancedSection::Players
                            && app.nbt_documents.active(key).map(|(id, _)| id) == first.active
                    }) {
                        return open_first(app);
                    }
                }
                Err(error) => {
                    log::error!("[player-browser] {error}");
                    app.player_browser.status = error;
                }
            }
        }
        Action::Page(delta) => {
            let max = app.player_browser.catalog.as_ref().map_or(0, |catalog| {
                app.player_browser
                    .filtered(catalog)
                    .len()
                    .div_ceil(32)
                    .saturating_sub(1)
            });
            app.player_browser.page = app
                .player_browser
                .page
                .saturating_add_signed(delta as isize)
                .min(max);
        }
        Action::Filter(value) => {
            app.player_browser.filter = value;
            app.player_browser.page = 0;
        }
        Action::Jump(target) => return jump(app, target, false),
        Action::View(target) => return jump(app, target, true),
        Action::Profile(target) => {
            let request = (|| -> Result<_, String> {
                target.get(app)?;
                let uuid = target.catalog.players[target.player]
                    .uuid
                    .ok_or("此玩家来源没有实际 UUID")?;
                if app.player_browser.profile_pending.is_some() {
                    return Err("正在读取前一名玩家的资料，请稍后再试".into());
                }
                Ok(uuid)
            })();
            let uuid = match request {
                Ok(uuid) => uuid,
                Err(error) => {
                    log::error!("[player-browser] {error}");
                    app.player_browser.status = error;
                    return Task::none();
                }
            };
            let epoch = app.player_browser.epoch;
            let world = target.world;
            let online = app.entity_models.online_skins;
            let cancel = Arc::clone(&app.player_browser.cancel);
            app.player_browser.profile_pending = Some(uuid);
            app.player_browser.status =
                format!("正在读取玩家资料：{}", player_catalog::uuid_string(uuid));
            return Task::perform(
                crate::nbt_background(move || {
                    if cancel.load(Ordering::Acquire) {
                        return Err("玩家资料读取已取消".into());
                    }
                    let profile = if online {
                        crate::player_profile::load(uuid)?
                    } else {
                        crate::player_profile::read_cached(uuid)?
                            .ok_or("没有有效的缓存姓名；可开启在线读取后查询")?
                    };
                    if cancel.load(Ordering::Acquire) {
                        return Err("玩家资料读取已取消".into());
                    }
                    Ok(profile)
                }),
                move |result| {
                    Message::PlayerBrowser(Action::ProfileLoaded(epoch, world, uuid, result))
                },
            );
        }
        Action::ProfileLoaded(epoch, world, uuid, result) => {
            if epoch != app.player_browser.epoch
                || app.player_browser.key != Some(world)
                || Some(world) != app.current_key().map(|key| key.0)
                || app.player_browser.profile_pending != Some(uuid)
            {
                return Task::none();
            }
            app.player_browser.profile_pending = None;
            if !app.entity_models.online_skins
                && result.as_ref().is_ok_and(|profile| !profile.cached)
            {
                app.player_browser.status = "在线玩家资料读取已关闭".into();
                return Task::none();
            }
            match result {
                Ok(profile) => {
                    app.player_browser.status = format!(
                        "已读取姓名：{}（{}）",
                        profile.name,
                        if profile.cached {
                            "本地公共资料缓存"
                        } else {
                            "Mojang 公共资料"
                        }
                    );
                    app.player_browser.profiles.insert(uuid, profile.name);
                }
                Err(error) => {
                    log::error!("[player-browser] {error}");
                    app.player_browser.status = error;
                }
            }
        }
        Action::Open(target, intent) => {
            app.player_browser.first = None;
            let result = (|| {
                if app.readonly && matches!(intent, Open::Move | Open::Rotate) {
                    return Err("只读模式禁止暂存玩家修改".to_owned());
                }
                let source = target.get(app)?;
                let prefix = if source.nbt_path == "/" {
                    String::new()
                } else {
                    format!("/{}", source.nbt_path.trim_matches('/'))
                };
                let focus = match intent {
                    Open::Attributes => prefix.clone(),
                    Open::Inventory => format!("{prefix}/Inventory"),
                    Open::Ender => format!("{prefix}/EnderItems"),
                    Open::Move => format!("{prefix}/Pos"),
                    Open::Rotate => format!("{prefix}/Rotation"),
                };
                let prefill = match intent {
                    Open::Move | Open::Rotate => crate::nbt_documents::Prefill::camera(
                        source.sha256.clone(),
                        prefix.clone(),
                        crate::player_editor::capture_camera(app, matches!(intent, Open::Move))?,
                        focus,
                    ),
                    _ => crate::nbt_documents::Prefill::new(
                        Some(source.sha256.clone()),
                        Vec::new(),
                        Some(focus),
                    ),
                };
                let path = source.path.clone();
                app.nbt_path = path.to_string_lossy().into_owned();
                app.nbt_editable = !app.readonly;
                app.nbt_prefill = Some(prefill);
                app.show_advanced = true;
                app.advanced_section = AdvancedSection::Players;
                Ok(path)
            })();
            match result {
                Ok(path) => {
                    app.player_browser.expanded = false;
                    return crate::nbt_documents::queue_open(app, path);
                }
                Err(error) => {
                    log::error!("[player-browser] {error}");
                    app.player_browser.status = error;
                }
            }
        }
    }
    Task::none()
}
