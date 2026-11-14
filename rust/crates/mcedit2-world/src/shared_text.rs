//! 薄引用共享文本；长度留在共享对象里，不随每个体素重复保存。
use std::{borrow::Borrow, fmt, ops::Deref, sync::Arc};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct SharedText(Arc<Box<str>>);
impl SharedText {
    /// 一次所有权统计内的分配身份，不保活源，不作为持久化键。
    pub(crate) fn allocation_id(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }
    pub(crate) fn allocation_bytes(&self) -> usize {
        2 * std::mem::size_of::<usize>() + std::mem::size_of::<Box<str>>() + self.len()
    }
}
impl Deref for SharedText {
    type Target = str;
    fn deref(&self) -> &str {
        self.0.as_ref().as_ref()
    }
}
impl AsRef<str> for SharedText {
    fn as_ref(&self) -> &str {
        self
    }
}
impl Borrow<str> for SharedText {
    fn borrow(&self) -> &str {
        self
    }
}
impl From<&str> for SharedText {
    fn from(value: &str) -> Self {
        Self(Arc::new(value.into()))
    }
}
impl From<String> for SharedText {
    fn from(value: String) -> Self {
        Self(Arc::new(value.into_boxed_str()))
    }
}
impl fmt::Debug for SharedText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}
impl fmt::Display for SharedText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self)
    }
}
