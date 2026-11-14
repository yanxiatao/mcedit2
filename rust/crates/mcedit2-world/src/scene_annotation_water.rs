//! 同一 Reader / Session 代次捕获实际水接触；源区块与暂存方块沿现有读取路径。
use super::*;
use crate::{
    entity_water,
    sections::{Chunk, State},
};
use std::{collections::BTreeMap, sync::Arc};

pub(super) fn attach(
    reader: &mut Reader<'_>,
    annotations: &mut ChunkAnnotations,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), WorldError> {
    let mut chunks = BTreeMap::<(i32, i32), Arc<Chunk>>::new();
    for marker in &mut annotations.markers {
        if cancelled() {
            return Err(invalid("实体水接触读取已取消"));
        }
        let Some(id) = marker
            .id
            .as_deref()
            .filter(|id| marker.kind == Kind::Entity && entity_water::needs_contact(id))
        else {
            continue;
        };
        if marker.visual_error.is_some() {
            continue;
        }
        let result = (|| {
            let visual = marker
                .visual
                .as_deref()
                .ok_or_else(|| invalid("鱼类缺少实际源外观"))?;
            let tags = reader
                .world
                .world_data
                .water_tags(
                    visual
                        .source_version
                        .ok_or_else(|| invalid("鱼类缺少实际源版本"))?,
                )
                .map_err(invalid)?;
            entity_water::inspect(id, visual, marker.position, &tags, &mut |[x, y, z]| {
                if cancelled() {
                    return Err(invalid("实体水接触读取已取消"));
                }
                let key = (x.div_euclid(16), z.div_euclid(16));
                if let std::collections::btree_map::Entry::Vacant(entry) = chunks.entry(key) {
                    let source = if let Some(session) = reader.display {
                        session
                            .visible_chunk_base_shared_cached(&mut reader.terrain, key.0, key.1)?
                            .0
                    } else {
                        reader.terrain.read_chunk_shared(key.0, key.1)?
                    };
                    entry.insert(
                        source.ok_or_else(|| {
                            invalid(format!("鱼类流体上下文缺少实际区块 {key:?}"))
                        })?,
                    );
                }
                let chunk = &chunks[&key];
                // 已存在原生区块中省略的 Section 在游戏读取为全空气；缺区块则在上方报错。
                let source = chunk
                    .block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                    .cloned()
                    .unwrap_or(State {
                        name: "minecraft:air".into(),
                        properties: Vec::new(),
                    });
                Ok(if let Some(session) = reader.display {
                    session.display_state((x, y, z), &source)
                } else {
                    source
                })
            })
        })();
        if cancelled() {
            return Err(invalid("实体水接触读取已取消"));
        }
        match result {
            Ok(contact) => entity_water::set(
                Arc::make_mut(marker.visual.as_mut().expect("鱼类源外观已读取")),
                contact,
            ),
            Err(error) => {
                let error = format!("{} 的水接触无法解释：{error}", marker.path);
                log::error!("[entity-water] {error}");
                marker.visual_error = Some(error.clone());
                annotations.diagnostics.push(error);
            }
        }
    }
    Ok(())
}
