//! 注册表标签的追加 / 替换、嵌套、可选项和循环诊断；调用方提供实际可见资源层。
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
type TagSources = BTreeMap<String, Vec<Arc<[u8]>>>;
pub(crate) fn resolve(
    name: &str,
    required: bool,
    sources: &TagSources,
    active: &mut BTreeSet<String>,
    complete: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<BTreeSet<String>, String> {
    if let Some(value) = complete.get(name) {
        return Ok(value.clone());
    }
    let Some(layers) = sources.get(name) else {
        return if required {
            Err(format!("缺少必需注册标签 #{name}"))
        } else {
            Ok(BTreeSet::new())
        };
    };
    if !active.insert(name.into()) {
        return Err(format!("注册标签存在循环：#{name}"));
    }
    let mut values = BTreeSet::new();
    for bytes in layers.iter().rev() {
        let value: Value =
            serde_json::from_slice(bytes).map_err(|e| format!("注册标签 #{name}：{e}"))?;
        for entry in value
            .get("values")
            .and_then(Value::as_array)
            .ok_or("注册标签缺少 values 数组")?
        {
            let (id, required) = if let Some(id) = entry.as_str() {
                (id, true)
            } else {
                (
                    entry
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or("标签项缺少 id")?,
                    entry
                        .get("required")
                        .map(|v| v.as_bool().ok_or("required 必须为 Bool"))
                        .transpose()?
                        .unwrap_or(true),
                )
            };
            let (nested, id) = id.strip_prefix('#').map_or((false, id), |id| (true, id));
            let id = if id.contains(':') {
                id.to_owned()
            } else {
                format!("minecraft:{id}")
            };
            crate::world_data::registry_id(&id).map_err(|error| error.to_string())?;
            if nested {
                values.extend(resolve(&id, required, sources, active, complete)?);
            } else {
                values.insert(id);
            }
        }
        if value
            .get("replace")
            .map(|v| v.as_bool().ok_or("标签 replace 必须为 Bool"))
            .transpose()?
            .unwrap_or(false)
        {
            break;
        }
    }
    active.remove(name);
    complete.insert(name.into(), values.clone());
    Ok(values)
}
