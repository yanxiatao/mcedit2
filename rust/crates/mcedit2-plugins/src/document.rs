//! 独立结构与世界共用运行协议；只把结果交给实际结构文档的候选和历史。
use crate::{InspectorContext, RunOutput};
use mcedit2_world::{
    editing::{snapshot::Content, Bounds, EditWorkBudget, SelectionRegion},
    structure_document::{Document, Object, PreparedScript, ScriptSnapshot},
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Inspector {
    pub object: Object,
    pub context: InspectorContext,
}
pub fn capture_inspector_cancellable(
    document: &Document,
    index: usize,
    budget: EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> Result<(ScriptSnapshot, Inspector), String> {
    let source = document.content();
    let body = source
        .block_entities
        .get(index)
        .ok_or("所选结构方块实体已移除")?;
    let region = SelectionRegion::from(Bounds {
        minimum: body.position,
        maximum: body.position,
    });
    let snapshot = document
        .capture_script_cancellable(region, false, budget, &mut cancelled)
        .map_err(|e| e.to_string())?;
    let nbt = document
        .object(Object::BlockEntity(index))
        .map_err(|e| e.to_string())?
        .inspect("")
        .map_err(|e| e.to_string())?;
    let mut position = [0; 3];
    for axis in 0..3 {
        position[axis] = source.source_min[axis]
            .checked_add(body.position[axis])
            .ok_or("结构方块实体源坐标溢出")?;
    }
    Ok((
        snapshot,
        Inspector {
            object: Object::BlockEntity(index),
            context: InspectorContext {
                position,
                source_sha: format!("{:x}", Sha256::digest(&body.compound)),
                path: String::new(),
                nbt,
            },
        },
    ))
}
pub fn prepare_output_cancellable(
    document: &Document,
    snapshot: &ScriptSnapshot,
    inspector: Option<&Inspector>,
    output: &RunOutput,
    budget: EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Option<PreparedScript>, String> {
    output.validate_with_budget(budget)?;
    if inspector.is_some()
        && (output.structure.is_some() || !output.keeps_snapshot(snapshot.clipboard())?)
    {
        return Err("Inspector 不能修改冻结选区或返回结构".into());
    }
    if !snapshot.matches(document) {
        return Err("插件所属结构、修订或定义已改变".into());
    }
    if output.content.is_some() && output.blocks.is_none() {
        return Err("完整对象 / 群系差异必须同时返回捕获范围的方块结果".into());
    }
    let clip = snapshot.clipboard();
    if output.structure.is_none()
        && output.inspector_operation.is_none()
        && output.keeps_snapshot(clip)?
    {
        return Ok(None);
    }
    let prepared = if let Some(operation) = &output.inspector_operation {
        let inspector = inspector.ok_or("插件没有 Inspector 来源上下文")?;
        if output.structure.is_some() || !output.keeps_snapshot(clip)? {
            return Err("Inspector 不能同时修改选区或返回结构".into());
        }
        let Object::BlockEntity(index) = inspector.object else {
            return Err("插件 Inspector 仅接收真实方块实体".into());
        };
        let (_, current) = capture_inspector_cancellable(document, index, budget, &mut cancelled)?;
        if current.context != inspector.context {
            return Err("Inspector 方块实体来源已变化".into());
        }
        document.prepare_script_object_cancellable(
            snapshot,
            inspector.object,
            operation.operations(),
            budget,
            &mut cancelled,
        )
    } else if let Some(generated) = &output.structure {
        if !output.keeps_snapshot(clip)? {
            let original;
            let content = if let Some(content) = &output.content {
                content
            } else {
                original = Content::capture(clip).map_err(|error| error.to_string())?;
                &original
            };
            let prepared = document
                .prepare_script_composition_cancellable(
                    snapshot,
                    output.blocks.as_deref().ok_or("组合脚本缺少源方块结果")?,
                    content,
                    Arc::new(generated.clone()),
                    budget,
                    &mut cancelled,
                )
                .map_err(|error| error.to_string())?;
            return Ok(prepared.changed.then_some(prepared));
        }
        document.prepare_script_generated_cancellable(
            snapshot,
            Arc::new(generated.clone()),
            budget,
            &mut cancelled,
        )
    } else if let Some(blocks) = &output.blocks {
        let original;
        let content = if let Some(content) = &output.content {
            content
        } else {
            original = Content::capture(clip).map_err(|e| e.to_string())?;
            &original
        };
        document.prepare_script_difference_cancellable(
            snapshot,
            blocks,
            content,
            budget,
            &mut cancelled,
        )
    } else {
        return Ok(None);
    }
    .map_err(|e| e.to_string())?;
    Ok(prepared.changed.then_some(prepared))
}
