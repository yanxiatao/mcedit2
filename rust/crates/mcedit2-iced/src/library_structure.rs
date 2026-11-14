//! 资料库向独立结构的投递；用真实文档与已显示落点定位，不借用后台世界。
use super::{Action, State};
use crate::{App, Message};
use iced::{Element, Point, Size, Task};
use mcedit2_world::clipboard::BlockClipboard;
use std::{
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
};

#[derive(Debug, Clone)]
pub(crate) struct Ticket {
    pub id: u64,
    source: Arc<BlockClipboard>,
    revision: Arc<()>,
    input_epoch: u64,
}
impl Ticket {
    fn capture(app: &App, id: u64) -> Option<Self> {
        if crate::structure_workspace::active(app) != Some(id) {
            return None;
        }
        let entry = app.structures.files.get(&id)?;
        if !entry.history_editable() {
            return None;
        }
        let document = entry.document.as_ref()?;
        Some(Self {
            id,
            source: document.content(),
            revision: document.revision(),
            input_epoch: app.input_epoch,
        })
    }
    fn matches(&self, app: &App) -> bool {
        Self::capture(app, self.id).is_some_and(|current| {
            Arc::ptr_eq(&self.source, &current.source)
                && Arc::ptr_eq(&self.revision, &current.revision)
        })
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Hit {
    pub position: [i32; 3],
    pub face: crate::isometric::Face,
}
impl State {
    pub fn close_structure(&mut self, id: u64) {
        if self.owner == Some(crate::keyboard_bindings::Owner::Structure(id)) {
            self.cancel();
            self.report = "所属结构文档已关闭，资料库任务已取消".into();
        }
    }
}
pub(super) fn import_at(
    app: &mut App,
    id: u64,
    path: PathBuf,
    destination: Option<[i32; 3]>,
) -> Task<Message> {
    let Some(ticket) = Ticket::capture(app, id) else {
        return super::error(app, "请先确认当前候选或切换到可编辑结构文档");
    };
    let destination = match destination {
        Some(point) => point,
        None => match app.structures.files[&id].import_minimum() {
            Ok(point) => point,
            Err(error) => return super::error(app, error),
        },
    };
    let (epoch, cancel) = app.library.begin();
    app.library.owner = Some(crate::keyboard_bindings::Owner::Structure(id));
    app.library.report = "正在识别要放置到独立文档的结构…".into();
    Task::perform(
        crate::nbt_background({
            let path = path.clone();
            move || {
                if cancel.load(Ordering::Acquire) {
                    return Err("资料库导入已取消".into());
                }
                crate::file_input::classify(&path)
            }
        }),
        move |result| {
            Message::Library(Action::StructureClassified(
                epoch,
                ticket.clone(),
                path.clone(),
                destination,
                result,
            ))
        },
    )
}
pub(super) fn classified(
    app: &mut App,
    epoch: u64,
    ticket: Ticket,
    path: PathBuf,
    destination: [i32; 3],
    result: Result<crate::file_input::Kind, String>,
) -> Task<Message> {
    if epoch != app.library.epoch {
        return Task::none();
    }
    app.library.busy = false;
    app.library.owner = None;
    if super::modal(app) || !ticket.matches(app) {
        return super::error(app, "独立结构导入目标已切换、修改或关闭，请重新投递");
    }
    match result {
        Ok(crate::file_input::Kind::Structure(format)) => {
            app.library.report = "已将资料库原稿交给当前结构的放置工具；预览后确认。".into();
            crate::structure_workspace::update(
                app,
                ticket.id,
                crate::structure_workspace::Action::Import(
                    crate::structure_workspace::import::Action::File(path, format, destination),
                ),
            )
        }
        Ok(_) => super::error(app, "所选文件不是结构，请使用打开文件入口"),
        Err(error) => super::error(app, error),
    }
}
pub(super) fn drop(
    app: &mut App,
    drop: crate::drag_drop::Drop,
    ticket: Ticket,
    hit: Result<Hit, String>,
) -> Task<Message> {
    let result: Result<(PathBuf, [i32; 3]), String> = (|| {
        drop.proof.claim()?;
        if ticket.input_epoch != app.input_epoch || !ticket.matches(app) {
            return Err("独立结构拖放来源或输入归属已改变".into());
        }
        let hit = hit?;
        let (x, y, z) = hit.face.normal();
        let normal = [x, y, z];
        let moved =
            std::array::from_fn::<_, 3, _>(|axis| hit.position[axis].checked_add(normal[axis]));
        let [Some(x), Some(y), Some(z)] = moved else {
            return Err("结构投递位置溢出".into());
        };
        let crate::drag_drop::Payload::Structure(path) = drop.payload else {
            return Err("独立结构视图只接受结构文件投递".into());
        };
        Ok((path, [x, y, z]))
    })();
    match result {
        Ok((path, destination)) => import_at(app, ticket.id, path, Some(destination)),
        Err(error) => super::error(app, error),
    }
}
pub(crate) fn target<'a>(
    app: &App,
    id: u64,
    content: Element<'a, Message>,
    pick: impl Fn(Point, Size) -> Option<Hit> + 'a,
) -> Element<'a, Message> {
    let Some(ticket) = Ticket::capture(app, id) else {
        return content;
    };
    crate::drag_drop::target(
        &app.drag_drop,
        crate::drag_drop::Kind::Structure,
        content,
        move |drop, point, size| {
            Some(Message::Library(Action::StructureDrop(
                drop,
                ticket.clone(),
                pick(point, size).ok_or_else(|| {
                    "当前位置没有可投递的实际结构表面或切片格，请等待画面更新".into()
                }),
            )))
        },
    )
}
