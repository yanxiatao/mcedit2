//! 文档实际保留的操作顺序与当前游标；修订身份用于拒绝过期的跳转请求。
use std::sync::Arc;

/// 内存操作的稳定身份；克隆会话、撤销和重做保持身份，新分支生成新身份。
#[derive(Debug, Clone)]
pub struct StepId(Arc<()>);
impl StepId {
    pub(crate) fn is_shared(&self) -> bool {
        Arc::strong_count(&self.0) > 1
    }
    pub(crate) fn new() -> Self {
        Self(Arc::new(()))
    }
}
impl PartialEq for StepId {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for StepId {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub label: String,
    pub detail: String,
}
#[derive(Debug, Clone)]
pub struct Timeline {
    pub revision: Arc<()>,
    /// 已应用的操作数；零表示最早保留状态，不保证该状态等于磁盘内容。
    pub cursor: usize,
    /// 已应用操作在前，待重做操作按执行顺序在后。
    pub entries: Vec<Entry>,
}
