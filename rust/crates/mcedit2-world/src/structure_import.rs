//! 外部结构的原位候选；完整文档和一次历史先在私有副本中准备，确认时核对真实归属。
use super::*;
use crate::{editing::Bounds, structure_transform::Transform};

#[derive(Debug, Clone, Copy)]
pub struct ImportOptions {
    pub anchor: [i32; 3],
    pub transform: Transform,
    pub entities: bool,
    pub paste: PasteOptions,
}
#[derive(Debug, Clone)]
pub struct PreparedImport {
    original: Arc<BlockClipboard>,
    revision: Arc<()>,
    definitions: Option<Arc<crate::world_data::Snapshot>>,
    source: Arc<BlockClipboard>,
    candidate: Arc<Document>,
    pub bounds: Bounds,
    pub changed: bool,
    pub blocks: usize,
    pub block_entities: usize,
    pub entities: usize,
}
impl PreparedImport {
    pub fn matches(&self, document: &Document) -> bool {
        Arc::ptr_eq(&self.original, &document.current)
            && Arc::ptr_eq(&self.revision, &document.revision)
            && match (&self.definitions, &document.definitions) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
    pub fn candidate(&self) -> &Document {
        &self.candidate
    }
    pub fn summary(&self) -> String {
        format!(
            "导入候选：{} 个声明格、{} 个方块实体、{} 棵实体树，范围 {:?} 至 {:?}；{}",
            self.blocks,
            self.block_entities,
            self.entities,
            self.bounds.minimum,
            self.bounds.maximum,
            if self.changed {
                "确认后一次文档撤销，尚未保存。"
            } else {
                "没有实际内容变化。"
            }
        )
    }
}
impl Document {
    pub fn prepare_import_cancellable(
        &self,
        source: Arc<BlockClipboard>,
        options: ImportOptions,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedImport, WorldError> {
        self.require_editable()?;
        if cancelled() {
            return Err(invalid("结构导入已取消"));
        }
        if let Some(proof) = &source.source_file {
            proof.verify()?;
        }
        let mut clip = (*source).clone();
        if !options.entities {
            clip.entities.clear();
        }
        if !options.paste.include_biomes {
            clip.biomes = None;
            clip.numeric_biomes = None;
        }
        if let Some(definitions) = &self.definitions {
            clip.bind_paintings(definitions)?;
        }
        let clip =
            clip.resampled_with_budget_cancellable(options.transform, budget, &mut cancelled)?;
        let mut minimum = [0; 3];
        let mut maximum = [0; 3];
        for axis in 0..3 {
            minimum[axis] = options.anchor[axis]
                .checked_add(clip.offset[axis])
                .ok_or_else(|| invalid("结构导入位置溢出"))?;
            maximum[axis] = minimum[axis]
                .checked_add(i32::from(clip.size[axis]) - 1)
                .ok_or_else(|| invalid("结构导入范围溢出"))?;
        }
        // 独立文档使用自身的 u16 尺寸，不套用世界维度的 Y 上下限；paste 核对实际目标范围。
        let bounds = Bounds { minimum, maximum };
        let mut candidate = self.clone();
        let changed = candidate.paste(&clip, minimum, options.paste, budget, &mut cancelled)?;
        if changed {
            candidate
                .undo
                .last_mut()
                .expect("导入粘贴已经记录一条历史")
                .label = "导入结构".into();
        }
        if let Some(proof) = &source.source_file {
            proof.verify()?;
        }
        if cancelled() {
            return Err(invalid("结构导入已取消"));
        }
        Ok(PreparedImport {
            original: Arc::clone(&self.current),
            revision: self.revision(),
            definitions: self.definitions.clone(),
            source,
            candidate: Arc::new(candidate),
            bounds,
            changed,
            blocks: clip.mask.as_ref().map_or(clip.blocks.len(), |mask| {
                mask.iter().filter(|&&selected| selected).count()
            }),
            block_entities: clip.block_entities.len(),
            entities: clip.entities.len(),
        })
    }
    pub fn commit_import_cancellable(
        &mut self,
        prepared: &PreparedImport,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<bool, WorldError> {
        self.require_editable()?;
        if !prepared.matches(self) {
            return Err(invalid("导入候选的目标文档或定义已经改变，请重新预览"));
        }
        if cancelled() {
            return Err(invalid("结构导入已取消"));
        }
        if let Some(proof) = &prepared.source.source_file {
            proof.verify()?;
        }
        if cancelled() {
            return Err(invalid("结构导入已取消"));
        }
        *self = (*prepared.candidate).clone();
        Ok(prepared.changed)
    }
}
