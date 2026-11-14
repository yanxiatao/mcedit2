//! 展开树只生成当前窗口的控件，分支分页不改变编辑目标或任何原始索引。
use super::{nodes, order, reveal, scalar, Action, NodeRequest, Tree};
use iced::{
    widget::{button, column, container, mouse_area, row, text, tooltip, Space},
    Element,
};
use mcedit2_world::nbt_document::{Document, TreeRow, TREE_BRANCH_PAGE};

pub(super) fn view<'a>(tree: &'a Tree, document: &Document) -> (Element<'a, Action>, usize) {
    let page = match tree
        .cache
        .page(document, &tree.hierarchy, tree.offset, tree.order)
    {
        Ok(page) => page,
        Err(error) => return (text(format!("无法读取 NBT 树：{error}")).into(), 0),
    };
    let mut body = column![text(format!(
        "{} 行 · 第 {} / {} 页",
        page.total,
        tree.offset / order::WINDOW + 1,
        page.total.div_ceil(order::WINDOW).max(1)
    ))]
    .spacing(3);
    let revision = document.revision();
    for item in &page.rows {
        match item {
            TreeRow::Node(node) => {
                let entry = &node.entry;
                let expand = button(if node.children.is_some() {
                    if node.expanded {
                        "▾"
                    } else {
                        "▸"
                    }
                } else {
                    "·"
                })
                .width(28)
                .padding(3)
                .style(button::text)
                .on_press_maybe(
                    node.children
                        .filter(|&count| count > 0)
                        .map(|_| Action::Toggle(entry.path.clone())),
                );
                let name: Element<'_, Action> =
                    if let Some(editor) = nodes::name_editor(tree, document, &entry.path) {
                        editor
                    } else if node.field {
                        mouse_area(container(text(node.name.clone())).padding([0, 3]))
                            .on_press(Action::Select(entry.path.clone(), entry.kind))
                            .on_double_click(Action::BeginRename(NodeRequest::new(
                                &revision,
                                &entry.path,
                                entry.kind,
                            )))
                            .interaction(iced::mouse::Interaction::Pointer)
                            .into()
                    } else {
                        text(node.name.clone()).into()
                    };
                let value: Element<'_, Action> =
                    if let Some(editor) = scalar::editor(tree, document, &entry.path) {
                        editor
                    } else {
                        let value = text(format!(
                            "[{}] {}",
                            order::type_name(entry.kind),
                            entry.summary
                        ))
                        .size(12);
                        if matches!(entry.kind, 1..=6 | 8) {
                            mouse_area(container(value).padding([0, 3]))
                                .on_press(Action::Select(entry.path.clone(), entry.kind))
                                .on_double_click(Action::BeginInline(NodeRequest::new(
                                    &revision,
                                    &entry.path,
                                    entry.kind,
                                )))
                                .interaction(iced::mouse::Interaction::Pointer)
                                .into()
                        } else {
                            value.into()
                        }
                    };
                let label = button(row![name, value].spacing(8))
                    .padding([3, 5])
                    .style(if tree.selected.as_ref() == Some(&entry.path) {
                        button::primary
                    } else {
                        button::secondary
                    })
                    .on_press(Action::Select(entry.path.clone(), entry.kind));
                let label = tooltip(
                    label,
                    text(format!(
                        "{} · {} 字节{}{}",
                        if entry.path.is_empty() {
                            "/"
                        } else {
                            &entry.path
                        },
                        entry.bytes,
                        if node.field {
                            " · 双击字段名重命名"
                        } else {
                            ""
                        },
                        if matches!(entry.kind, 1..=6 | 8) {
                            " · 双击值编辑"
                        } else {
                            ""
                        }
                    )),
                    tooltip::Position::Top,
                );
                let line = container(
                    row![
                        expand,
                        button("+")
                            .padding([3, 5])
                            .on_press_maybe(node.children.is_some().then(|| Action::AddNode(
                                NodeRequest::new(&revision, &entry.path, entry.kind)
                            ))),
                        button("−")
                            .padding([3, 5])
                            .on_press_maybe((!entry.path.is_empty()).then(|| Action::DeleteNode(
                                NodeRequest::new(&revision, &entry.path, entry.kind)
                            ))),
                        label,
                    ]
                    .align_y(iced::Alignment::Center),
                );
                let line = if tree.reveal_path.as_ref() == Some(&entry.path) {
                    line.id(reveal::TARGET)
                } else {
                    line
                };
                body = body.push(row![Space::new().width(node.depth as f32 * 12.), line]);
            }
            TreeRow::BranchPage {
                path,
                depth,
                offset,
                total,
            } => {
                body = body.push(
                    row![
                        Space::new().width(*depth as f32 * 12. + 28.),
                        text(format!(
                            "子项 {} / {} 页",
                            offset / TREE_BRANCH_PAGE + 1,
                            total.div_ceil(TREE_BRANCH_PAGE).max(1)
                        ))
                        .size(12),
                        button("首项").on_press_maybe(
                            (*offset > 0).then(|| Action::BranchPage(path.clone(), 0))
                        ),
                        button("前一组").on_press_maybe((*offset > 0).then(|| Action::BranchPage(
                            path.clone(),
                            offset.saturating_sub(TREE_BRANCH_PAGE)
                        ))),
                        button("后一组").on_press_maybe(
                            (offset + TREE_BRANCH_PAGE < *total).then(|| Action::BranchPage(
                                path.clone(),
                                offset + TREE_BRANCH_PAGE
                            ))
                        ),
                    ]
                    .spacing(5)
                    .align_y(iced::Alignment::Center),
                );
            }
        }
    }
    (body.into(), page.total)
}
