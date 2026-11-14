//! 树行新增 / 删除绑定实际文档修订，不依赖显示顺序或当前编辑器中的旧路径。
use super::{order, reveal, Action, Tree};
use iced::{
    widget::{button, column, container, row, text, text_input},
    Element,
};
use mcedit2_world::nbt_document::{Document, Operation};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub(crate) struct Request {
    pub(super) revision: Arc<()>,
    pub(super) path: String,
    pub(super) kind: u8,
}
impl Request {
    pub(super) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.revision, &other.revision)
            && self.path == other.path
            && self.kind == other.kind
    }
    pub(super) fn new(revision: &Arc<()>, path: &str, kind: u8) -> Self {
        Self {
            revision: Arc::clone(revision),
            path: path.into(),
            kind,
        }
    }
    pub(super) fn verify(&self, document: &Document) -> Result<(), String> {
        if !Arc::ptr_eq(&self.revision, &document.revision()) {
            return Err("NBT 文档已修改，请使用当前树行重新操作".into());
        }
        let actual = document
            .node_info(&self.path)
            .map_err(|error| error.to_string())?;
        if actual.kind != self.kind {
            return Err("NBT 节点类型已改变，请重新选择".into());
        }
        Ok(())
    }
}
pub(super) fn add(
    tree: &mut Tree,
    document: &Document,
    request: Request,
    chosen: Option<u8>,
) -> Result<Option<Operation>, String> {
    request.verify(document)?;
    if chosen.is_some()
        && !tree.adding.as_ref().is_some_and(|open| {
            Arc::ptr_eq(&open.revision, &request.revision)
                && open.path == request.path
                && open.kind == request.kind
        })
    {
        return Err("新增类型选择已取消或目标已改变，请重新点击容器的新增按钮".into());
    }
    let fixed = document
        .child_type(&request.path)
        .map_err(|error| error.to_string())?;
    let Some(kind) = chosen.or(fixed) else {
        tree.renaming = None;
        tree.editing = None;
        tree.adding = Some(request);
        return Ok(None);
    };
    let operation = document
        .new_child(&request.path, kind)
        .map_err(|error| error.to_string())?;
    prepare_view(tree, document, request.path)?;
    Ok(Some(operation))
}
pub(super) fn remove(
    tree: &mut Tree,
    document: &Document,
    request: Request,
) -> Result<Operation, String> {
    request.verify(document)?;
    let parent = request
        .path
        .rsplit_once('/')
        .ok_or("根节点不能删除")?
        .0
        .to_owned();
    prepare_view(tree, document, parent)?;
    Ok(Operation::Remove { path: request.path })
}
fn prepare_view(tree: &mut Tree, document: &Document, path: String) -> Result<(), String> {
    let mut shown = document
        .reveal_tree(&tree.hierarchy, tree.order.source(), &path)
        .map_err(|error| error.to_string())?;
    shown.expanded.insert(path.clone());
    let index = document
        .tree_index(&shown, tree.order.source(), &path)
        .map_err(|error| error.to_string())?;
    tree.hierarchy = shown;
    tree.offset = index / order::WINDOW * order::WINDOW;
    tree.path = path.clone();
    tree.selected = None;
    tree.scalar.clear();
    tree.scalar_enabled = false;
    tree.json.clear();
    tree.name.clear();
    tree.index.clear();
    tree.notice.clear();
    tree.adding = None;
    tree.renaming = None;
    tree.editing = None;
    tree.rename_enabled = false;
    tree.request_reveal(Some(path));
    Ok(())
}
pub(super) struct Rename {
    request: Request,
    name: String,
}
pub(super) fn begin_rename(
    tree: &mut Tree,
    document: &Document,
    request: Request,
) -> Result<(), String> {
    request.verify(document)?;
    let name = document
        .field_name(&request.path)
        .map_err(|error| error.to_string())?
        .ok_or("根和列表 / 数组元素不能重命名")?;
    tree.show(request.path.clone(), request.kind, document, false)?;
    tree.edit_ticket = tree
        .edit_ticket
        .checked_add(1)
        .expect("NBT 改名输入代次溢出");
    tree.renaming = Some(Rename { request, name });
    Ok(())
}
pub(super) fn rename_text(
    tree: &mut Tree,
    document: &Document,
    request: Request,
    name: String,
) -> Result<(), String> {
    request.verify(document)?;
    let pending = tree
        .renaming
        .as_mut()
        .filter(|pending| pending.request.same(&request))
        .ok_or("字段重命名已取消或目标已改变")?;
    pending.name = name;
    Ok(())
}
pub(super) fn commit_rename(
    tree: &mut Tree,
    document: &Document,
    request: Request,
) -> Result<Option<Operation>, String> {
    request.verify(document)?;
    let pending = tree
        .renaming
        .as_ref()
        .filter(|pending| pending.request.same(&request))
        .ok_or("字段重命名已取消或目标已改变")?;
    let operation = document
        .rename_field(&request.path, &pending.name)
        .map_err(|error| error.to_string())?;
    if operation.is_some() {
        let parent = request
            .path
            .rsplit_once('/')
            .expect("改名目标是实际字段")
            .0
            .to_owned();
        prepare_view(tree, document, parent)?;
    } else {
        tree.renaming = None;
    }
    Ok(operation)
}
pub(super) fn rename_selected(
    tree: &mut Tree,
    document: &Document,
) -> Result<Option<Operation>, String> {
    let path = tree.selected.as_ref().ok_or("请先选择字段")?;
    let operation = document
        .rename_field(path, &tree.name)
        .map_err(|error| error.to_string())?;
    if operation.is_some() {
        let parent = path
            .rsplit_once('/')
            .expect("改名目标是实际字段")
            .0
            .to_owned();
        prepare_view(tree, document, parent)?;
    }
    Ok(operation)
}
pub(super) fn rename_view<'a>(tree: &'a Tree, document: &Document) -> Element<'a, Action> {
    let Some(pending) = &tree.renaming else {
        return column![].into();
    };
    if Arc::ptr_eq(&pending.request.revision, &document.revision()) {
        return column![].into();
    }
    column![
        text("文档已修改，请从当前字段重新开始重命名。"),
        button("取消改名").on_press(Action::CancelRename)
    ]
    .into()
}
pub(super) fn name_editor<'a>(
    tree: &'a Tree,
    document: &Document,
    path: &str,
) -> Option<Element<'a, Action>> {
    let pending = tree
        .renaming
        .as_ref()
        .filter(|pending| pending.request.path == path)?;
    if !Arc::ptr_eq(&pending.request.revision, &document.revision()) {
        return None;
    }
    let request = pending.request.clone();
    Some(
        container(
            row![
                text_input("字段名", &pending.name)
                    .id(reveal::RENAME)
                    .width(200)
                    .on_input(move |value| Action::RenameText(request.clone(), value))
                    .on_submit(Action::CommitRename(pending.request.clone())),
                button("✓").on_press(Action::CommitRename(pending.request.clone())),
                button("×").on_press(Action::CancelRename),
            ]
            .spacing(3),
        )
        .id(reveal::EDITOR)
        .into(),
    )
}
pub(super) fn chooser<'a>(tree: &'a Tree, document: &Document) -> Element<'a, Action> {
    let Some(request) = &tree.adding else {
        return column![].into();
    };
    let current = Arc::ptr_eq(&request.revision, &document.revision());
    let mut body = column![text(format!(
        "新增节点到：{}",
        if request.path.is_empty() {
            "/"
        } else {
            &request.path
        }
    ))]
    .spacing(4);
    if current {
        let mut kinds = row![].spacing(3);
        for kind in 1..=12 {
            kinds = kinds.push(
                button(order::type_name(kind)).on_press(Action::AddType(request.clone(), kind)),
            );
        }
        body = body.push(kinds.wrap());
    } else {
        body = body.push(text("文档已修改，请重新点击目标容器的新增按钮。"));
    }
    body.push(button("取消新增").on_press(Action::CancelAdd))
        .into()
}
