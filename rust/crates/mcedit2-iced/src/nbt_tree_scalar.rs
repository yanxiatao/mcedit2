//! 标量行内编辑沿真实类型解析，旧行与已取消编辑不能投递到其他节点。
use super::{reveal, Action, NodeRequest, Tree};
use iced::{
    widget::{button, column, container, row, text, text_input},
    Element,
};
use mcedit2_world::nbt_document::{Document, Operation, Value};
use std::sync::Arc;

pub(super) struct Editor {
    request: NodeRequest,
    text: String,
}
pub(super) fn begin(
    tree: &mut Tree,
    document: &Document,
    request: NodeRequest,
) -> Result<(), String> {
    request.verify(document)?;
    if !matches!(request.kind, 1..=6 | 8) {
        return Err("此节点不是可行内编辑的标量".into());
    }
    let value = document
        .inspect(&request.path)
        .map_err(|error| error.to_string())?;
    let text = crate::world_info::as_text(&value).ok_or("此节点不是可行内编辑的标量")?;
    tree.show(request.path.clone(), request.kind, document, false)?;
    tree.edit_ticket = tree
        .edit_ticket
        .checked_add(1)
        .expect("NBT 行内输入代次溢出");
    tree.editing = Some(Editor { request, text });
    Ok(())
}
pub(super) fn input(
    tree: &mut Tree,
    document: &Document,
    request: NodeRequest,
    text: String,
) -> Result<(), String> {
    request.verify(document)?;
    let editor = tree
        .editing
        .as_mut()
        .filter(|editor| editor.request.same(&request))
        .ok_or("值编辑已取消或目标已改变")?;
    editor.text = text;
    Ok(())
}
pub(super) fn commit(
    tree: &mut Tree,
    document: &Document,
    request: NodeRequest,
) -> Result<Option<Operation>, String> {
    request.verify(document)?;
    let editor = tree
        .editing
        .as_ref()
        .filter(|editor| editor.request.same(&request))
        .ok_or("值编辑已取消或目标已改变")?;
    let previous = document
        .inspect(&request.path)
        .map_err(|error| error.to_string())?;
    let value = crate::world_info::parse_like(previous.clone(), &editor.text)?;
    value.payload().map_err(|error| error.to_string())?;
    let unchanged = match (&previous, &value) {
        (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
        (Value::Double(a), Value::Double(b)) => a.to_bits() == b.to_bits(),
        _ => previous == value,
    };
    if unchanged {
        tree.editing = None;
        return Ok(None);
    }
    let json = serde_json::to_string(&value).map_err(|error| error.to_string())?;
    tree.scalar = editor.text.clone();
    tree.json = json;
    tree.editing = None;
    Ok(Some(Operation::Replace {
        path: request.path,
        value,
    }))
}
pub(super) fn editor<'a>(
    tree: &'a Tree,
    document: &Document,
    path: &str,
) -> Option<Element<'a, Action>> {
    let editor = tree
        .editing
        .as_ref()
        .filter(|editor| editor.request.path == path)?;
    if !Arc::ptr_eq(&editor.request.revision, &document.revision()) {
        return None;
    }
    let request = editor.request.clone();
    Some(
        container(
            row![
                text_input("标量值", &editor.text)
                    .id(reveal::VALUE)
                    .width(240)
                    .on_input(move |text| Action::InlineText(request.clone(), text))
                    .on_submit(Action::CommitInline(editor.request.clone())),
                button("✓").on_press(Action::CommitInline(editor.request.clone())),
                button("×").on_press(Action::CancelInline),
            ]
            .spacing(3),
        )
        .id(reveal::EDITOR)
        .into(),
    )
}
pub(super) fn status<'a>(tree: &'a Tree, document: &Document) -> Element<'a, Action> {
    let Some(editor) = &tree.editing else {
        return column![].into();
    };
    if Arc::ptr_eq(&editor.request.revision, &document.revision()) {
        return column![].into();
    }
    column![
        text("文档已修改，请从当前节点重新开始编辑值。"),
        button("取消值编辑").on_press(Action::CancelInline)
    ]
    .into()
}
