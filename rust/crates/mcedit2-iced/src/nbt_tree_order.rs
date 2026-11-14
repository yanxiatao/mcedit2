//! NBT 树的显示顺序和按实际文档修订缓存的行窗口。
use mcedit2_world::nbt_document::{ChildOrder, Document, TreePage, TreeState};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Order {
    Stored,
    #[default]
    NameAscending,
    NameDescending,
    TypeAscending,
    TypeDescending,
}
impl Order {
    pub(super) const ALL: [Self; 5] = [
        Self::Stored,
        Self::NameAscending,
        Self::NameDescending,
        Self::TypeAscending,
        Self::TypeDescending,
    ];
    pub(super) fn source(self) -> ChildOrder {
        match self {
            Self::Stored => ChildOrder::Stored,
            Self::NameAscending => ChildOrder::NameAscending,
            Self::NameDescending => ChildOrder::NameDescending,
            Self::TypeAscending => ChildOrder::TypeAscending,
            Self::TypeDescending => ChildOrder::TypeDescending,
        }
    }
}
impl std::fmt::Display for Order {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Stored => "原始存储顺序",
            Self::NameAscending => "名称升序（容器优先）",
            Self::NameDescending => "名称降序",
            Self::TypeAscending => "标签类型升序",
            Self::TypeDescending => "标签类型降序",
        })
    }
}
pub(super) fn type_name(kind: u8) -> &'static str {
    match kind {
        1 => "Byte",
        2 => "Short",
        3 => "Int",
        4 => "Long",
        5 => "Float",
        6 => "Double",
        7 => "Byte[]",
        8 => "String",
        9 => "List",
        10 => "Compound",
        11 => "Int[]",
        12 => "Long[]",
        _ => unreachable!("实际 NBT 节点类型必须为 1..=12"),
    }
}
pub(super) const WINDOW: usize = 128;
struct Saved {
    revision: Arc<()>,
    hierarchy: TreeState,
    offset: usize,
    order: Order,
    result: Result<Arc<TreePage>, String>,
}
#[derive(Default)]
pub(super) struct Cache(Mutex<Option<Saved>>);
impl Cache {
    pub(super) fn page(
        &self,
        document: &Document,
        hierarchy: &TreeState,
        offset: usize,
        order: Order,
    ) -> Result<Arc<TreePage>, String> {
        let revision = document.revision();
        let mut cache = self.0.lock().expect("NBT 显示分页缓存锁被毒化");
        if let Some(saved) = cache.as_ref().filter(|saved| {
            Arc::ptr_eq(&saved.revision, &revision)
                && &saved.hierarchy == hierarchy
                && saved.offset == offset
                && saved.order == order
        }) {
            return saved.result.clone();
        }
        let result = document
            .tree_page(hierarchy, order.source(), offset, WINDOW)
            .map(Arc::new)
            .map_err(|error| error.to_string());
        if let Err(error) = &result {
            log::error!("[nbt-tree] {error}");
        }
        *cache = Some(Saved {
            revision,
            hierarchy: hierarchy.clone(),
            offset,
            order,
            result: result.clone(),
        });
        result
    }
}
