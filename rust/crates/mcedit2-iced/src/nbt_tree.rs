//! 文件与对象 Inspector 共用的 NBT 树控件；编辑宿主决定读写和历史权限。
use iced::{
    widget::{button, column, pick_list, row, text, text_input},
    Element,
};
use mcedit2_world::nbt_document::{Document, Operation, TreeState, Value};
#[path = "nbt_tree_hierarchy.rs"]
mod hierarchy;
#[path = "nbt_tree_nodes.rs"]
mod nodes;
#[path = "nbt_tree_order.rs"]
mod order;
#[path = "nbt_tree_scroll.rs"]
mod reveal;
#[path = "nbt_tree_scalar.rs"]
mod scalar;
pub(crate) use nodes::Request as NodeRequest;
pub(crate) use order::Order;
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Select(String, u8),
    Parent,
    Previous,
    Next,
    First,
    Toggle(String),
    BranchPage(String, usize),
    CollapseAll,
    Order(Order),
    AddNode(NodeRequest),
    AddType(NodeRequest, u8),
    DeleteNode(NodeRequest),
    CancelAdd,
    BeginRename(NodeRequest),
    RenameText(NodeRequest, String),
    CommitRename(NodeRequest),
    CancelRename,
    BeginInline(NodeRequest),
    InlineText(NodeRequest, String),
    CommitInline(NodeRequest),
    CancelInline,
    Json(String),
    ApplyBatch,
    Scalar(String),
    ApplyScalar,
    Name(String),
    Index(String),
    Template(u8),
    Replace,
    InsertField,
    InsertItem,
    Remove,
    Rename,
    Undo,
    Redo,
}
pub(crate) enum Command {
    None,
    Edit(Vec<Operation>),
    Undo,
    Redo,
}
#[derive(Default)]
pub(crate) struct Tree {
    path: String,
    selected: Option<String>,
    offset: usize,
    json: String,
    name: String,
    index: String,
    scalar: String,
    scalar_enabled: bool,
    notice: String,
    order: Order,
    cache: order::Cache,
    hierarchy: TreeState,
    reveal_ticket: u64,
    reveal_path: Option<String>,
    identity: std::sync::Arc<()>,
    adding: Option<NodeRequest>,
    renaming: Option<nodes::Rename>,
    rename_enabled: bool,
    edit_ticket: u64,
    editing: Option<scalar::Editor>,
}
impl Tree {
    /// 外部查找定位展开实际祖先及各层所在页，保留选中的容器行。
    pub(crate) fn focus(
        &mut self,
        path: String,
        kind: u8,
        document: &Document,
    ) -> Result<(), String> {
        self.show(path, kind, document, false)
    }
    fn show(
        &mut self,
        path: String,
        kind: u8,
        document: &Document,
        enter: bool,
    ) -> Result<(), String> {
        let mut shown = document
            .reveal_tree(&self.hierarchy, self.order.source(), &path)
            .map_err(|e| e.to_string())?;
        if enter && matches!(kind, 7 | 9 | 10 | 11 | 12) {
            shown.expanded.insert(path.clone());
        }
        let index = document
            .tree_index(&shown, self.order.source(), &path)
            .map_err(|e| e.to_string())?;
        self.select(path.clone(), kind, document, enter)?;
        self.hierarchy = shown;
        self.offset = index / order::WINDOW * order::WINDOW;
        self.adding = None;
        self.renaming = None;
        self.editing = None;
        self.request_reveal(Some(path));
        Ok(())
    }
    fn request_reveal(&mut self, path: Option<String>) {
        self.reveal_ticket = self
            .reveal_ticket
            .checked_add(1)
            .expect("NBT 树导航代次溢出");
        self.reveal_path = path;
    }
    fn select(
        &mut self,
        path: String,
        kind: u8,
        document: &Document,
        enter: bool,
    ) -> Result<(), String> {
        let info = document.node_info(&path).map_err(|e| e.to_string())?;
        if info.kind != kind {
            return Err("所选节点类型已改变，请重新选择".into());
        }
        let (scalar, json, notice) = if info.bytes <= mcedit2_world::nbt_document::MAX_VALUE_BYTES {
            let value = document.inspect(&path).map_err(|e| e.to_string())?;
            (
                crate::world_info::as_text(&value),
                serde_json::to_string(&value).map_err(|e| e.to_string())?,
                String::new(),
            )
        } else {
            (
                None,
                String::new(),
                format!(
                    "节点 {} 字节，超过整体值编辑预算；{}也可明确填写新的类型化 JSON。",
                    info.bytes,
                    if matches!(kind, 7 | 9 | 10 | 11 | 12) {
                        "可进入后按页编辑子节点；"
                    } else {
                        ""
                    }
                ),
            )
        };
        let field_name = document
            .field_name(&path)
            .map_err(|error| error.to_string())?;
        let rename_enabled = field_name.is_some();
        let index = if field_name.is_none() {
            path.rsplit_once('/')
                .map(|(_, name)| name.to_owned())
                .unwrap_or_default()
        } else {
            String::new()
        };
        if enter && matches!(kind, 7 | 9 | 10 | 11 | 12) {
            self.path = path.clone();
        } else {
            self.path = path
                .rsplit_once('/')
                .map_or("", |(parent, _)| parent)
                .into();
        }
        self.selected = Some(path);
        self.scalar_enabled = scalar.is_some();
        self.scalar = scalar.unwrap_or_default();
        self.json = json;
        self.notice = notice;
        self.name = field_name.unwrap_or_default();
        self.index = index;
        self.rename_enabled = rename_enabled;
        Ok(())
    }
    pub(crate) fn update(
        &mut self,
        action: Action,
        document: &Document,
    ) -> Result<Command, String> {
        let mut operation = None;
        match action {
            Action::Select(path, kind) => {
                self.show(path, kind, document, true)?;
            }
            Action::Parent => {
                if let Some((parent, _)) = self.path.rsplit_once('/') {
                    let parent = parent.to_owned();
                    let kind = document.node_info(&parent).map_err(|e| e.to_string())?.kind;
                    self.show(parent, kind, document, true)?;
                }
            }
            Action::Previous => {
                self.offset = self.offset.saturating_sub(order::WINDOW);
                self.request_reveal(None);
            }
            Action::First => {
                self.offset = 0;
                self.request_reveal(None);
            }
            Action::Next => {
                let page = self
                    .cache
                    .page(document, &self.hierarchy, self.offset, self.order)?;
                if self.offset + order::WINDOW < page.total {
                    self.offset += order::WINDOW;
                    self.request_reveal(None);
                }
            }
            Action::Toggle(path) => {
                let kind = document.node_info(&path).map_err(|e| e.to_string())?.kind;
                if !matches!(kind, 7 | 9 | 10 | 11 | 12) {
                    return Err("该节点不是可展开容器".into());
                }
                let mut shown = self.hierarchy.clone();
                if !shown.expanded.remove(&path) {
                    shown.expanded.insert(path.clone());
                }
                let index = document
                    .tree_index(&shown, self.order.source(), &path)
                    .map_err(|e| e.to_string())?;
                self.hierarchy = shown;
                self.offset = index / order::WINDOW * order::WINDOW;
                self.request_reveal(Some(path));
            }
            Action::BranchPage(path, offset) => {
                let kind = document.node_info(&path).map_err(|e| e.to_string())?.kind;
                if !matches!(kind, 7 | 9 | 10 | 11 | 12) {
                    return Err("该节点没有分支页".into());
                }
                let mut shown = self.hierarchy.clone();
                shown.expanded.insert(path.clone());
                shown.pages.insert(path.clone(), offset);
                let index = document
                    .tree_index(&shown, self.order.source(), &path)
                    .map_err(|e| e.to_string())?;
                self.hierarchy = shown;
                self.offset = index / order::WINDOW * order::WINDOW;
                self.request_reveal(Some(path));
            }
            Action::CollapseAll => {
                self.hierarchy.expanded.clear();
                self.hierarchy.pages.clear();
                self.offset = 0;
                self.request_reveal(Some(String::new()));
            }
            Action::Order(order) => {
                let (shown, offset) = if let Some(path) = &self.selected {
                    let shown = document
                        .reveal_tree(&self.hierarchy, order.source(), path)
                        .map_err(|e| e.to_string())?;
                    let index = document
                        .tree_index(&shown, order.source(), path)
                        .map_err(|e| e.to_string())?;
                    (shown, index / order::WINDOW * order::WINDOW)
                } else {
                    (self.hierarchy.clone(), 0)
                };
                self.hierarchy = shown;
                self.order = order;
                self.offset = offset;
                self.request_reveal(self.selected.clone());
            }
            Action::Json(value) => self.json = value,
            Action::AddNode(request) => {
                operation = nodes::add(self, document, request, None)?;
            }
            Action::AddType(request, kind) => {
                operation = nodes::add(self, document, request, Some(kind))?;
            }
            Action::DeleteNode(request) => {
                operation = Some(nodes::remove(self, document, request)?);
            }
            Action::CancelAdd => self.adding = None,
            Action::BeginRename(request) => nodes::begin_rename(self, document, request)?,
            Action::RenameText(request, value) => {
                nodes::rename_text(self, document, request, value)?
            }
            Action::CommitRename(request) => {
                operation = nodes::commit_rename(self, document, request)?;
            }
            Action::CancelRename => self.renaming = None,
            Action::BeginInline(request) => scalar::begin(self, document, request)?,
            Action::InlineText(request, text) => scalar::input(self, document, request, text)?,
            Action::CommitInline(request) => {
                operation = scalar::commit(self, document, request)?;
            }
            Action::CancelInline => self.editing = None,
            Action::Scalar(value) => self.scalar = value,
            Action::ApplyScalar => {
                let path = self.selected.clone().ok_or("请先选择节点")?;
                let value = crate::world_info::parse_like(
                    document.inspect(&path).map_err(|e| e.to_string())?,
                    &self.scalar,
                )?;
                operation = Some(Operation::Replace { path, value });
            }
            Action::Name(value) => self.name = value,
            Action::Index(value) => self.index = value,
            Action::Template(kind) => {
                let value = Value::new_tag(kind).map_err(|error| error.to_string())?;
                self.json = serde_json::to_string(&value).map_err(|e| e.to_string())?;
            }
            Action::ApplyBatch => {
                let operations = serde_json::from_str::<Vec<Operation>>(&self.json)
                    .map_err(|e| format!("批量操作需要 Operation JSON 数组：{e}"))?;
                return Ok(Command::Edit(operations));
            }
            Action::Replace => {
                operation = Some(Operation::Replace {
                    path: self.selected.clone().ok_or("请先选择节点")?,
                    value: self.value()?,
                })
            }
            Action::InsertField => {
                operation = Some(Operation::InsertField {
                    path: self.path.clone(),
                    name: self.name.clone(),
                    value: self.value()?,
                })
            }
            Action::InsertItem => {
                operation = Some(Operation::InsertItem {
                    path: self.path.clone(),
                    index: self.index.parse().map_err(|_| "请输入非负列表索引")?,
                    value: self.value()?,
                })
            }
            Action::Remove => {
                operation = Some(Operation::Remove {
                    path: self.selected.clone().ok_or("请先选择节点")?,
                });
                self.selected = None;
                self.rename_enabled = false;
                self.renaming = None;
                self.editing = None;
                self.scalar_enabled = false;
            }
            Action::Rename => {
                operation = nodes::rename_selected(self, document)?;
            }
            Action::Undo => return Ok(Command::Undo),
            Action::Redo => return Ok(Command::Redo),
        }
        Ok(match operation {
            Some(operation) => Command::Edit(vec![operation]),
            None => Command::None,
        })
    }
    fn value(&self) -> Result<Value, String> {
        serde_json::from_str(&self.json).map_err(|e| format!("类型化 JSON 无效：{e}"))
    }
    pub(crate) fn view<'a>(&'a self, document: &Document) -> Element<'a, Action> {
        let (tree, total) = hierarchy::view(self, document);
        let mut templates = row![].spacing(3);
        for (id, name) in [
            (1, "Byte"),
            (2, "Short"),
            (3, "Int"),
            (4, "Long"),
            (5, "Float"),
            (6, "Double"),
            (7, "Byte[]"),
            (8, "String"),
            (9, "List"),
            (10, "Compound"),
            (11, "Int[]"),
            (12, "Long[]"),
        ] {
            templates = templates.push(button(text(name)).on_press(Action::Template(id)));
        }
        let content = column![
            row![
                button("选择根").on_press(Action::Select(String::new(), 10)),
                button("上级").on_press_maybe((!self.path.is_empty()).then_some(Action::Parent)),
                button("折叠全部").on_press(Action::CollapseAll),
                button("树首页").on_press_maybe((self.offset > 0).then_some(Action::First)),
                button("上一页").on_press_maybe((self.offset > 0).then_some(Action::Previous)),
                button("下一页")
                    .on_press_maybe((self.offset + order::WINDOW < total).then_some(Action::Next))
            ]
            .spacing(6)
            .wrap(),
            pick_list(Order::ALL, Some(self.order), Action::Order),
            text(format!(
                "编辑容器：{}",
                if self.path.is_empty() {
                    "/"
                } else {
                    &self.path
                }
            ))
            .size(12),
            text(format!(
                "选中节点：{}",
                self.selected
                    .as_deref()
                    .map(|path| if path.is_empty() { "/" } else { path })
                    .unwrap_or("未选择")
            ))
            .size(12),
            reveal::view(tree),
            nodes::chooser(self, document),
            nodes::rename_view(self, document),
            scalar::status(self, document),
            templates.wrap(),
            row![
                text_input("选中节点的值", &self.scalar).on_input(Action::Scalar),
                button("应用值").on_press_maybe(self.scalar_enabled.then_some(Action::ApplyScalar))
            ]
            .spacing(5)
            .wrap(),
            text("类型化 JSON：列表、复合节点与数组可进入后逐项编辑"),
            text(&self.notice).size(12),
            text_input("类型化 JSON 或批量操作数组", &self.json).on_input(Action::Json),
            button("批量应用 JSON（一次撤销）").on_press(Action::ApplyBatch),
            text("批量操作的路径及索引均指向修改前的文档；任一错误将整组拒绝。").size(12),
            row![
                text_input("字段名", &self.name).on_input(Action::Name),
                text_input("列表索引", &self.index).on_input(Action::Index)
            ]
            .spacing(6),
            row![
                button("替换选中").on_press(Action::Replace),
                button("添加字段").on_press(Action::InsertField),
                button("插入列表项").on_press(Action::InsertItem),
                button("删除选中").on_press(Action::Remove),
                button("重命名").on_press_maybe(self.rename_enabled.then_some(Action::Rename))
            ]
            .spacing(5)
            .wrap(),
            row![
                button("撤销文档修改").on_press(Action::Undo),
                button("重做文档修改").on_press(Action::Redo)
            ]
            .spacing(5)
            .wrap(),
        ]
        .spacing(6)
        .into();
        reveal::wrap(self, content)
    }
}
