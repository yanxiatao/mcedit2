//! 字段名来自真实 Compound；改名沿原始 Rename 操作保留节点载荷。
use super::{
    children::child_position, parse_path, parse_root, refuse, resolve, string, ChildOrder,
    Document, Operation, Result,
};
impl Document {
    /// 返回实际字段名；根和列表 / 数组元素没有可修改的 Compound 字段名。
    pub fn field_name(&self, path: &str) -> Result<Option<String>> {
        let root = parse_root(&self.bytes).map_err(|error| refuse(error.to_string()))?;
        let mut parts = parse_path(path)?;
        let Some(name) = parts.pop() else {
            return Ok(None);
        };
        let parent = resolve(&root, &parts)?;
        child_position(parent, &name, ChildOrder::Stored)?;
        Ok((parent.kind == 10).then_some(name))
    }
    /// 构造字段改名，重名和编码错误正常报告；同名不产生编辑操作。
    pub fn rename_field(&self, path: &str, name: &str) -> Result<Option<Operation>> {
        let root = parse_root(&self.bytes).map_err(|error| refuse(error.to_string()))?;
        let mut parts = parse_path(path)?;
        let previous = parts
            .pop()
            .ok_or_else(|| refuse("根节点没有可修改的字段名"))?;
        let parent = resolve(&root, &parts)?;
        if parent.kind != 10 {
            return Err(refuse("列表和数组元素不能重命名"));
        }
        child_position(parent, &previous, ChildOrder::Stored)?;
        if previous == name {
            return Ok(None);
        }
        if parent.fields.iter().any(|(field, _)| field == name) {
            return Err(refuse("新字段名已存在"));
        }
        string(name, &mut Vec::new())?;
        Ok(Some(Operation::Rename {
            path: path.into(),
            name: name.into(),
        }))
    }
}
