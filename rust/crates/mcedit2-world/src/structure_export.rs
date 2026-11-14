//! 原版结构导出的作者与排除状态；只处理导出副本，不改来源或编辑历史。
use super::*;
use crate::{
    editing::parse_state,
    nbt_document::{self, Operation, Value},
    sections::{parse_root, State},
};
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub entities: bool,
    pub biomes: bool,
    /// None 保留已有元数据；Some 明确写入作者，包括空字符串。
    pub author: Option<String>,
    pub excluded: Vec<State>,
}
impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            entities: true,
            biomes: true,
            author: None,
            excluded: Vec::new(),
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct ExportSummary {
    pub blocks: usize,
    pub block_entities: usize,
    pub entities: usize,
    pub excluded: usize,
}
fn invalid(message: impl ToString) -> WorldError {
    WorldError::Invalid(message.to_string())
}
fn stop(cancelled: &mut impl FnMut() -> bool) -> Result<(), WorldError> {
    if cancelled() {
        Err(invalid("结构导出已取消"))
    } else {
        Ok(())
    }
}
impl ExportOptions {
    fn apply(
        &self,
        format: Format,
        clip: &mut BlockClipboard,
        budget: EditWorkBudget,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<usize, WorldError> {
        if !self.entities {
            clip.entities.clear();
        }
        if !self.biomes {
            clip.biomes = None;
            clip.numeric_biomes = None;
        }
        if format != Format::Structure && (self.author.is_some() || !self.excluded.is_empty()) {
            return Err(invalid("作者和排除状态选项只适用于原版结构 NBT 导出"));
        }
        let mut excluded = 0;
        if !self.excluded.is_empty() {
            for state in &self.excluded {
                if parse_state(&crate::editing::render_state(state))? != *state {
                    return Err(invalid("排除状态必须是规范的完整方块状态"));
                }
                if state.name.starts_with("numeric:") != clip.numeric {
                    return Err(invalid("排除状态的数字 / 命名布局与结构不同"));
                }
            }
            let states: HashSet<_> = self.excluded.iter().collect();
            budget.check(
                clip.work_bytes()?.saturating_mul(4)
                    + clip.blocks.len()
                    + self
                        .excluded
                        .iter()
                        .map(|state| clipboard::state_bytes(state) + 96)
                        .sum::<usize>(),
            )?;
            let mask = clip
                .mask
                .get_or_insert_with(|| vec![true; clip.blocks.len()]);
            for (index, state) in clip.blocks.iter().enumerate() {
                if index.is_multiple_of(4096) {
                    stop(cancelled)?;
                }
                if mask[index] && states.contains(state) {
                    mask[index] = false;
                    excluded += 1;
                }
            }
            let [width, _, depth] = clip.size;
            clip.block_entities.retain(|entity| {
                let [x, y, z] = entity.position;
                mask[(y as usize * usize::from(depth) + z as usize) * usize::from(width)
                    + x as usize]
            });
            clip.scheduled_ticks.retain(|tick| {
                let [x, y, z] = tick.position;
                mask[(y as usize * usize::from(depth) + z as usize) * usize::from(width)
                    + x as usize]
            });
        }
        if let Some(author) = &self.author {
            let mut raw = vec![10, 0, 0];
            raw.extend_from_slice(clip.metadata.as_deref().unwrap_or(&[0]));
            let root = parse_root(&raw).map_err(invalid)?;
            let operation = if root.field("author").is_some() {
                Operation::Replace {
                    path: "/author".into(),
                    value: Value::String(author.clone()),
                }
            } else {
                Operation::InsertField {
                    path: String::new(),
                    name: "author".into(),
                    value: Value::String(author.clone()),
                }
            };
            let patched = nbt_document::patch(&raw, &operation).map_err(invalid)?;
            clip.metadata = Some(patched[3..].to_vec());
        }
        stop(cancelled)?;
        Ok(excluded)
    }
}
impl Format {
    /// 完整编码与来源核对后才创建输出文件；实际写盘开始后按真实 I/O 结果返回。
    pub fn export_cancellable(
        self,
        context: &impl crate::structure_context::CodecContext,
        source: &BlockClipboard,
        path: &Path,
        options: &ExportOptions,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<ExportSummary, WorldError> {
        stop(&mut cancelled)?;
        if !path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| self.extensions().contains(&extension))
        {
            return Err(invalid("导出扩展名与选择的结构格式不符"));
        }
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        if context.exports_into_world(parent)? {
            return Err(invalid("结构文件必须导出到世界目录之外"));
        }
        if let Some(proof) = &source.source_file {
            proof.verify()?;
        }
        source.validate_with_budget(context, &source.dimension, budget)?;
        let mut clip = source.clone();
        clip.source_bound = false;
        let excluded = options.apply(self, &mut clip, budget, &mut cancelled)?;
        let bytes = self.encode_with_budget(context, &clip, budget)?;
        stop(&mut cancelled)?;
        if let Some(proof) = &source.source_file {
            proof.verify()?;
        }
        write_new(path, &bytes)?;
        Ok(ExportSummary {
            blocks: clip.mask.as_ref().map_or(clip.blocks.len(), |mask| {
                mask.iter().filter(|&&selected| selected).count()
            }),
            block_entities: clip.block_entities.len(),
            entities: clip.entities.len(),
            excluded,
        })
    }
}
