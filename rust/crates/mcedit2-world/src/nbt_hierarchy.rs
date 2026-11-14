//! 按展开状态投影原始 NBT 层级；一次解析、分支分页、全树窗口化，不复制节点载荷。
use super::children::{child_position, compound_order, length, sequence_index};
use super::{
    array_item, entry, parse_path, parse_root, refuse, resolve, ChildOrder, Document, Entry,
    Result, Tag,
};
use std::collections::{BTreeMap, BTreeSet};

/// 单个容器在树内展示的实际直接子项数量。
pub const TREE_BRANCH_PAGE: usize = 64;
/// 展开路径与每个实际容器的直接子项偏移，只作为只读视图参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeState {
    pub expanded: BTreeSet<String>,
    pub pages: BTreeMap<String, usize>,
}
impl Default for TreeState {
    fn default() -> Self {
        Self {
            expanded: BTreeSet::from([String::new()]),
            pages: BTreeMap::new(),
        }
    }
}
#[derive(Debug, Clone)]
/// 单个实际节点的层级、摘要与容器信息；编辑仍使用 entry.path。
pub struct TreeNode {
    pub entry: Entry,
    pub name: String,
    pub depth: usize,
    pub children: Option<usize>,
    pub expanded: bool,
    pub field: bool,
}
#[derive(Debug, Clone)]
/// 实际节点行或其所属容器的分页控件行，分页行不是 NBT 节点。
pub enum TreeRow {
    Node(TreeNode),
    BranchPage {
        path: String,
        depth: usize,
        offset: usize,
        total: usize,
    },
}
#[derive(Debug, Clone)]
/// 指定展开树的可见行窗口及完整行数。
pub struct TreePage {
    pub rows: Vec<TreeRow>,
    pub total: usize,
}
struct Projection<'a> {
    state: &'a TreeState,
    order: ChildOrder,
    offset: usize,
    limit: usize,
    target: Option<&'a str>,
    found: Option<usize>,
    page: TreePage,
}
impl Projection<'_> {
    fn locate(&mut self, path: &str) -> Result<()> {
        if self.target == Some(path) {
            if self.found.is_some() {
                return Err(refuse("NBT 目标字段路径重复"));
            }
            self.found = Some(self.page.total);
        }
        Ok(())
    }
    fn node(
        &mut self,
        tag: &Tag<'_>,
        path: String,
        name: String,
        depth: usize,
        field: bool,
    ) -> Result<()> {
        let children = if matches!(tag.kind, 7 | 9 | 10 | 11 | 12) {
            Some(length(tag)?)
        } else {
            None
        };
        let expanded = children.is_some() && self.state.expanded.contains(&path);
        self.locate(&path)?;
        if self.page.total >= self.offset && self.page.rows.len() < self.limit {
            self.page.rows.push(TreeRow::Node(TreeNode {
                entry: entry(tag, path.clone()),
                name,
                depth,
                children,
                expanded,
                field,
            }));
        }
        self.page.total += 1;
        if !expanded {
            return Ok(());
        }
        let total = children.expect("展开节点是实际容器");
        let offset = self.state.pages.get(&path).copied().unwrap_or(0);
        if total > TREE_BRANCH_PAGE || offset > 0 {
            if self.page.total >= self.offset && self.page.rows.len() < self.limit {
                self.page.rows.push(TreeRow::BranchPage {
                    path: path.clone(),
                    depth: depth + 1,
                    offset,
                    total,
                });
            }
            self.page.total += 1;
        }
        let indices = (tag.kind == 10 && self.order != ChildOrder::Stored)
            .then(|| compound_order(tag, self.order));
        for ordinal in offset..total.min(offset.saturating_add(TREE_BRANCH_PAGE)) {
            let source = if tag.kind == 10 {
                indices.as_ref().map_or(ordinal, |indices| indices[ordinal])
            } else {
                sequence_index(ordinal, total, self.order)
            };
            match tag.kind {
                10 => {
                    let (name, child) = &tag.fields[source];
                    self.node(
                        child,
                        format!("{path}/{}", name.replace('~', "~0").replace('/', "~1")),
                        name.clone(),
                        depth + 1,
                        true,
                    )?;
                }
                9 => self.node(
                    &tag.items[source],
                    format!("{path}/{source}"),
                    format!("[{source}]"),
                    depth + 1,
                    false,
                )?,
                7 | 11 | 12 => {
                    let child_path = format!("{path}/{source}");
                    self.locate(&child_path)?;
                    if self.page.total >= self.offset && self.page.rows.len() < self.limit {
                        let value = array_item(tag, source)?;
                        self.page.rows.push(TreeRow::Node(TreeNode {
                            entry: Entry {
                                path: child_path,
                                kind: value.kind(),
                                bytes: match value.kind() {
                                    1 => 1,
                                    3 => 4,
                                    _ => 8,
                                },
                                summary: format!("{value:?}"),
                            },
                            name: format!("[{source}]"),
                            depth: depth + 1,
                            children: None,
                            expanded: false,
                            field: false,
                        }));
                    }
                    self.page.total += 1;
                }
                _ => unreachable!("展开节点是实际容器"),
            }
        }
        Ok(())
    }
}
impl Document {
    fn project_tree<'a>(
        &self,
        state: &'a TreeState,
        order: ChildOrder,
        offset: usize,
        limit: usize,
        target: Option<&'a str>,
    ) -> Result<Projection<'a>> {
        if limit == 0 || limit > 256 {
            return Err(refuse("NBT 树窗口大小必须为 1..=256"));
        }
        let root = parse_root(&self.bytes).map_err(|error| refuse(error.to_string()))?;
        let mut projection = Projection {
            state,
            order,
            offset,
            limit,
            target,
            found: None,
            page: TreePage {
                rows: Vec::new(),
                total: 0,
            },
        };
        projection.node(&root, String::new(), "/".into(), 0, false)?;
        Ok(projection)
    }
    /// 只为所需的可见行窗口生成摘要；未进入窗口的分支仅计算行数与真实位置。
    pub fn tree_page(
        &self,
        state: &TreeState,
        order: ChildOrder,
        offset: usize,
        limit: usize,
    ) -> Result<TreePage> {
        Ok(self.project_tree(state, order, offset, limit, None)?.page)
    }
    /// 目标必须位于当前实际展开的分支页中；不把不可见节点猜作其他行。
    pub fn tree_index(&self, state: &TreeState, order: ChildOrder, path: &str) -> Result<usize> {
        self.project_tree(state, order, 0, 1, Some(path))?
            .found
            .ok_or_else(|| refuse("目标节点未在当前展开页中显示"))
    }
    /// 展开目标的全部实际祖先，并为每层容器选择包含目标的页，保留其他分支状态。
    pub fn reveal_tree(
        &self,
        state: &TreeState,
        order: ChildOrder,
        path: &str,
    ) -> Result<TreeState> {
        let root = parse_root(&self.bytes).map_err(|error| refuse(error.to_string()))?;
        let parts = parse_path(path)?;
        let mut shown = state.clone();
        let mut parent_path = String::new();
        for (depth, child) in parts.iter().enumerate() {
            let parent = resolve(&root, &parts[..depth])?;
            let position = child_position(parent, child, order)?;
            shown.expanded.insert(parent_path.clone());
            shown.pages.insert(
                parent_path.clone(),
                position / TREE_BRANCH_PAGE * TREE_BRANCH_PAGE,
            );
            parent_path.push('/');
            parent_path.push_str(&child.replace('~', "~0").replace('/', "~1"));
        }
        if parts.is_empty() {
            shown.expanded.insert(String::new());
        }
        Ok(shown)
    }
}
