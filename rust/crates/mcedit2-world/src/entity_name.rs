//! 实体保存名称的版本适配；只展开实际文本，不生成实体类型名或执行文本事件。
use crate::{entity_catalog::EntityVisual, nbt_document::Value};
#[derive(Debug, Clone)]
pub struct Source {
    pub component: serde_json::Value,
    pub always_visible: bool,
}
impl Source {
    /// profile_name 仅接收已按真实 UUID 取得的玩家资料名称，不用于覆盖损坏的 CustomName。
    pub fn read(visual: &EntityVisual, profile_name: Option<&str>) -> Result<Option<Self>, String> {
        let Some(value) = visual.appearance.get("CustomName") else {
            return Ok(profile_name.map(|name| Self {
                component: serde_json::Value::String(name.into()),
                always_visible: true,
            }));
        };
        let Value::String(name) = value else {
            return Err("CustomName 必须为 String".into());
        };
        let component = match visual.source_version {
            Some(1343) => {
                if name.is_empty() {
                    return Ok(None);
                }
                serde_json::Value::String(name.clone())
            }
            Some(3465 | 3955) => {
                serde_json::from_str(name).map_err(|e| format!("CustomName 文本组件无效：{e}"))?
            }
            version => return Err(format!("未核实源版本 {version:?} 的实体名称格式")),
        };
        let always_visible = match visual.appearance.get("CustomNameVisible") {
            None => false,
            Some(Value::Byte(n)) => *n != 0,
            _ => return Err("CustomNameVisible 必须为 Byte".into()),
        };
        Ok(Some(Self {
            component,
            always_visible,
        }))
    }
}
