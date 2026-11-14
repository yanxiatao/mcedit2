//! 同一 Reader 维度定义进入实体显示副本；错误沿场景诊断报告，不猜测维度。
use super::*;
use crate::entity_zombification;
use std::sync::Arc;

pub(super) fn attach(
    reader: &Reader<'_>,
    annotations: &mut ChunkAnnotations,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), WorldError> {
    for marker in &mut annotations.markers {
        if cancelled() {
            return Err(invalid("实体转化上下文读取已取消"));
        }
        if marker.kind != Kind::Entity
            || !marker
                .id
                .as_deref()
                .is_some_and(entity_zombification::supports)
            || marker.visual_error.is_some()
        {
            continue;
        }
        let result = (|| {
            let visual = marker
                .visual
                .as_deref()
                .ok_or_else(|| invalid("转化实体缺少源外观"))?;
            let version = visual
                .source_version
                .ok_or_else(|| invalid("转化实体缺少实际源版本"))?;
            let safe = if entity_zombification::needs_context(visual).map_err(invalid)? {
                Some(
                    reader
                        .world
                        .world_data
                        .piglin_safe(reader.dimension, version)?,
                )
            } else {
                None
            };
            let loved = if marker.id.as_deref() == Some("minecraft:piglin") {
                Some(
                    match crate::piglin_appearance::offhand(visual).map_err(invalid)? {
                        Some(item) => reader
                            .world
                            .world_data
                            .piglin_tags(version)
                            .map_err(invalid)?
                            .loves(&item),
                        None => false,
                    },
                )
            } else {
                None
            };
            Ok::<_, WorldError>((safe, loved))
        })();
        match result {
            Ok((safe, loved)) => {
                let visual = Arc::make_mut(marker.visual.as_mut().expect("转化实体源外观已读取"));
                if let Some(safe) = safe {
                    entity_zombification::set(visual, safe);
                }
                if let Some(loved) = loved {
                    crate::piglin_appearance::set(visual, loved);
                }
            }
            Err(error) => {
                let error = format!("{} 的转化上下文无法解释：{error}", marker.path);
                log::error!("[entity-zombification] {error}");
                marker.visual_error = Some(error.clone());
                annotations.diagnostics.push(error);
            }
        }
    }
    Ok(())
}
