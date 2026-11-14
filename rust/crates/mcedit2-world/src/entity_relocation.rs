//! 按原始根定位、跨槽移动与完整 NBT 批次改写；由私有组合会话统一发布。
use super::*;
use crate::nbt_document::Value;

fn list_path(source: &EntityChange) -> Result<&'static str, WorldError> {
    let root =
        crate::sections::parse_root(&source.before).map_err(|error| refuse(error.to_string()))?;
    Ok(
        if !source.source.starts_with("entity-region:") && root.field("Level").is_some() {
            "/Level/Entities"
        } else {
            "/Entities"
        },
    )
}
impl Session {
    /// 调用者持有私有会话：后段错误丢弃整个组合，不发布中间跨槽移动。
    pub(super) fn rewrite_structure_entity_cancellable(
        &mut self,
        before: &clipboard::OpaqueEntity,
        after: Option<&Value>,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<(), WorldError> {
        if cancelled() {
            return Err(refuse("实体变换重放已取消"));
        }
        let original = snapshot::value(&before.compound)?;
        if after == Some(&original) {
            return Ok(());
        }
        let source = self.structure_entity_source(before)?;
        let input = self.entity_input(source.chunk)?;
        let list = list_path(&input)?;
        let mut path = format!("{list}/{}", source.index);
        let Some(after) = after else {
            let preview = self.prepare_entity_delete(source.chunk, &path)?;
            self.apply_entity_preview(preview)?;
            return Ok(());
        };
        let position = snapshot::entity_position(after)?;
        let chunk = [position[0], position[2]].map(|value| (value / 16.).floor());
        if chunk
            .iter()
            .any(|value| *value < f64::from(i32::MIN) || *value > f64::from(i32::MAX))
        {
            return Err(refuse("实体变换后的区块坐标越界"));
        }
        let to = (chunk[0] as i32, chunk[1] as i32);
        let mut current = original;
        if to != source.chunk {
            let delta = [
                to.0.checked_sub(source.chunk.0)
                    .and_then(|n| n.checked_mul(16)),
                Some(0),
                to.1.checked_sub(source.chunk.1)
                    .and_then(|n| n.checked_mul(16)),
            ];
            let [Some(x), Some(y), Some(z)] = delta else {
                return Err(refuse("实体跨槽位移溢出"));
            };
            let preview = self.prepare_entity_move(source.chunk, to, source.index, [x, y, z])?;
            self.apply_entity_preview(preview)?;
            let target = self.entity_input(to)?;
            let list = list_path(&target)?;
            let root = crate::sections::parse_root(&target.before)
                .map_err(|error| refuse(error.to_string()))?;
            let parent = if list == "/Level/Entities" {
                root.field("Level").expect("已核实 Level")
            } else {
                &root
            };
            let index = parent
                .require("Entities", 9)
                .map_err(|error| refuse(error.to_string()))?
                .items
                .len()
                .checked_sub(1)
                .ok_or_else(|| refuse("跨槽移动后缺少目标实体根"))?;
            path = format!("{list}/{index}");
            current = snapshot::value(&crate::nbt_document::raw_compound_list_item(
                &target.before,
                list,
                index,
            )?)?;
        }
        if cancelled() {
            return Err(refuse("实体变换重放已取消"));
        }
        let mut operations = Vec::new();
        snapshot::diff(&current, after, &path, &mut operations);
        if !operations.is_empty() {
            let mut input = self.entity_input(to)?;
            let proof = crate::entity_editing::edit_batches(
                &input.before,
                to,
                input.source.starts_with("entity-region:"),
                &path,
                vec![operations],
            )?;
            input.after = proof.replay(&input.before)?;
            self.apply_entity_preview(EntityPreview {
                root: self.world.root.clone(),
                dimension: self.dimension.clone(),
                identity_revision: None,
                changes: vec![input],
                proof: EntityProof::EditBatch(proof),
            })?;
        }
        Ok(())
    }
}
