//! 插件 Inspector 的真实暂存方块实体上下文和源身份。
use crate::scene_layers::MarkerRef;
use mcedit2_plugins::InspectorContext;
use mcedit2_world::{editing::Session, nbt_document::Value, scene_annotations::Kind};

pub(crate) fn inspector(
    session: &Session,
    selected: &MarkerRef,
) -> Result<InspectorContext, String> {
    let marker = selected.marker();
    if marker.kind != Kind::BlockEntity {
        return Err("请选择方块实体".into());
    }
    let p = marker.position;
    if p.iter()
        .any(|v| !v.is_finite() || v.fract() != 0. || *v < i32::MIN as f64 || *v > i32::MAX as f64)
    {
        return Err("方块实体位置无效".into());
    }
    let pos = (p[0] as i32, p[1] as i32, p[2] as i32);
    let original = session
        .inspect_original_block_entity(pos)
        .map_err(|e| e.to_string())?
        .ok_or("方块实体原始源不存在")?;
    if original.original_compound_sha256 != marker.sha256 {
        return Err("方块实体原始源已改变，请重新读取".into());
    }
    let (doc, path, source_sha) = session.staged_be_document(pos).map_err(|e| e.to_string())?;
    let nbt = doc.inspect(&path).map_err(|e| e.to_string())?;
    if !matches!(&nbt, Value::Compound(_)) {
        return Err("方块实体不是 NBT Compound".into());
    }
    fn finite(value: &Value) -> bool {
        match value {
            Value::Float(v) => v.is_finite(),
            Value::Double(v) => v.is_finite(),
            Value::List(list) => list.items.iter().all(finite),
            Value::Compound(fields) => fields.iter().all(|field| finite(&field.value)),
            _ => true,
        }
    }
    if !finite(&nbt) {
        return Err("方块实体包含非有限浮点，JSON 插件协议无法无损表示".into());
    }
    Ok(InspectorContext {
        position: [pos.0, pos.1, pos.2],
        source_sha,
        path,
        nbt,
    })
}

pub(crate) fn prefix_operation(
    mut operation: mcedit2_world::nbt_document::Operation,
    prefix: &str,
) -> Result<mcedit2_world::nbt_document::Operation, String> {
    use mcedit2_world::nbt_document::Operation;
    let path = match &mut operation {
        Operation::Replace { path, .. }
        | Operation::InsertField { path, .. }
        | Operation::InsertItem { path, .. }
        | Operation::Remove { path }
        | Operation::Rename { path, .. } => path,
    };
    if !path.is_empty() && !path.starts_with('/') {
        return Err("Inspector NBT 路径必须相对于方块实体，以 / 开头".into());
    }
    *path = format!("{prefix}{path}");
    Ok(operation)
}
