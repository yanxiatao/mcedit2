//! 新文档的证明在私有基线上重放；发布前不借用或编造任何 GUI 文档编号。
use super::*;

/// 真实加载基线与最终会话一起交接，保留内容历史、权限和选区 / 视图 / 保存意图。
#[derive(Debug, Clone)]
pub struct OpenedOutput {
    pub baseline: OpenedDocument,
    pub document: OpenedDocument,
    pub selections: BTreeMap<String, Option<SelectionRegion>>,
    pub focus: Option<Focus>,
    pub save: bool,
}
impl OpenedOutput {
    pub fn slot(&self) -> usize {
        self.baseline.slot()
    }
    /// 文档身份由外层持有；新回调按实际目录重排槽位时，只改槽位字段。
    pub fn with_slot(mut self, slot: usize) -> Self {
        for document in [&mut self.baseline, &mut self.document] {
            match document {
                OpenedDocument::World { slot: current, .. }
                | OpenedDocument::Structure { slot: current, .. } => *current = slot,
            }
        }
        if let Some(focus) = &mut self.focus {
            focus.world = slot;
        }
        self
    }
    pub fn work_bytes(&self) -> Result<usize, WorldError> {
        document_bytes(&self.baseline)?
            .checked_add(document_bytes(&self.document)?)
            .ok_or_else(|| refuse("私有新文档完整工作集溢出"))
    }
    /// 后续结果已经从上一份实际私有会话重放；整段工具继续折回首次加载基线的一次历史。
    pub fn follow(
        &self,
        mut following: Self,
        budget: EditWorkBudget,
        cancelled: &AtomicBool,
    ) -> Result<Self, WorldError> {
        let previous = self.clone().with_slot(following.slot());
        match (
            &previous.document,
            &following.baseline,
            &previous.baseline,
            &mut following.document,
        ) {
            (
                OpenedDocument::World { input: current, .. },
                OpenedDocument::World { input: source, .. },
                OpenedDocument::World {
                    input: original, ..
                },
                OpenedDocument::World { input: target, .. },
            ) => {
                if current.world.root != source.world.root
                    || current.readonly != source.readonly
                    || current.sessions.len() != source.sessions.len()
                {
                    return Err(refuse("私有世界后续结果的实际来源不同"));
                }
                for (name, before) in &current.sessions {
                    let source = source
                        .sessions
                        .get(name)
                        .ok_or_else(|| refuse("私有世界后续结果缺少来源维度"))?;
                    if !Arc::ptr_eq(&before.revision(), &source.revision()) {
                        return Err(refuse("私有世界后续结果不属于前序会话修订"));
                    }
                    if cancelled.load(Ordering::Acquire) {
                        return Err(refuse("私有新世界候选合并已取消"));
                    }
                    let baseline = original
                        .sessions
                        .get(name)
                        .ok_or_else(|| refuse("私有世界首次基线缺少实际维度"))?;
                    let session = target
                        .sessions
                        .get_mut(name)
                        .ok_or_else(|| refuse("私有世界后续结果缺少最终维度"))?;
                    if Arc::ptr_eq(&before.revision(), &session.revision()) {
                        continue;
                    }
                    *session = match baseline.finish_script_candidate(session.clone(), budget)? {
                        Some(prepared) => prepared.candidate().clone(),
                        None => baseline.clone(),
                    };
                }
            }
            (
                OpenedDocument::Structure {
                    document: current, ..
                },
                OpenedDocument::Structure {
                    document: source, ..
                },
                OpenedDocument::Structure {
                    document: original, ..
                },
                OpenedDocument::Structure {
                    document: target, ..
                },
            ) => {
                if !current.same_source(source) {
                    return Err(refuse("私有结构后续结果不属于前序会话修订"));
                }
                if !current.same_source(target) {
                    *target = original.finish_startup(target, budget)?;
                }
            }
            _ => return Err(refuse("私有新文档的前后实际类别不同")),
        }
        following.baseline = previous.baseline;
        following.focus = following.focus.or(previous.focus);
        following.save |= previous.save;
        if cancelled.load(Ordering::Acquire) {
            return Err(refuse("私有新文档候选合并已取消"));
        }
        Ok(following)
    }
}
fn document_bytes(document: &OpenedDocument) -> Result<usize, WorldError> {
    match document {
        OpenedDocument::Structure { document, .. } => document.content().work_bytes(),
        OpenedDocument::World { input, .. } => {
            let mut bytes = input.world.world_data.allocated_bytes();
            for session in input.sessions.values() {
                bytes = bytes
                    .checked_add(std::mem::size_of::<Session>())
                    .and_then(|bytes| {
                        bytes.checked_add(
                            input
                                .world
                                .dimensions
                                .len()
                                .checked_mul(std::mem::size_of::<crate::DimensionSummary>())?,
                        )
                    })
                    .ok_or_else(|| refuse("私有世界目录工作集溢出"))?;
                for group in session.undo.iter().chain(session.redo.iter()) {
                    bytes = bytes
                        .checked_add(compound::group_bytes(group)?)
                        .ok_or_else(|| refuse("私有世界历史证明工作集溢出"))?;
                }
            }
            Ok(bytes)
        }
    }
}
impl Working {
    /// 续接目录中的私有文档仍占用脚本预算，但不冒充本回调刚打开的文档。
    pub fn retain_private_documents(
        &mut self,
        documents: &[OpenedOutput],
    ) -> Result<(), WorldError> {
        let bytes = documents.iter().try_fold(0usize, |sum, document| {
            sum.checked_add(document.work_bytes()?)
                .ok_or_else(|| refuse("私有文档合计工作集溢出"))
        })?;
        self.opened_bytes = self.reserve_document(bytes)?;
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct ResolvedOutput {
    /// 不含私有新文档的结果；维度表保留完整实际槽位，不重新排序。
    pub existing: Output,
    /// 按本回调实际槽位排列；稳定身份与首次打开顺序由调用目录持有。
    pub opened: Vec<OpenedOutput>,
}
impl Output {
    /// 消费整组结果，任一新文档的证明、来源或取消失败都会丢弃本次整理。
    /// 世界使用与正式会话相同的历史重放，结构沿用 finish_startup 的真实文档结果。
    pub fn resolve_opened(
        self,
        cancelled: &AtomicBool,
        progress: impl FnMut(EditWorkProgress),
    ) -> Result<ResolvedOutput, WorldError> {
        self.resolve_private(Vec::new(), cancelled, progress)
    }
    /// 前序私有文档可以位于本回调的任意真实槽位；与新加载来源一起整理。
    pub fn resolve_private(
        mut self,
        private: Vec<OpenedOutput>,
        cancelled: &AtomicBool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<ResolvedOutput, WorldError> {
        let stop = || {
            if cancelled.load(Ordering::Acquire) {
                Err(refuse("脚本新文档结果整理已取消"))
            } else {
                Ok(())
            }
        };
        stop()?;
        let mut opened = BTreeMap::new();
        for prior in private {
            let slot = prior.slot();
            let document = prior.document;
            if opened
                .insert(
                    slot,
                    OpenedOutput {
                        baseline: document.clone(),
                        document,
                        selections: prior.selections,
                        focus: None,
                        save: false,
                    },
                )
                .is_some()
            {
                return Err(refuse("私有文档目录重复登记同一真实槽位"));
            }
        }
        for baseline in std::mem::take(&mut self.opened) {
            let slot = baseline.slot();
            let document = baseline.clone();
            let selections = match &document {
                OpenedDocument::World { input, .. } => input.selections.clone(),
                OpenedDocument::Structure { document, .. } => {
                    BTreeMap::from([(document.content().dimension.clone(), None)])
                }
            };
            if opened
                .insert(
                    slot,
                    OpenedOutput {
                        baseline,
                        document,
                        selections,
                        focus: None,
                        save: false,
                    },
                )
                .is_some()
            {
                return Err(refuse("新加载文档与前序私有文档占用了同一槽位"));
            }
        }
        for (&slot, result) in &mut opened {
            let dimension = self
                .dimensions
                .get(slot)
                .ok_or_else(|| refuse("私有文档不在实际目录中"))?;
            match &mut result.document {
                OpenedDocument::World { input, .. } => {
                    if !input.sessions.contains_key(dimension) {
                        return Err(refuse("私有世界最终维度没有实际会话"));
                    }
                    input.dimension = dimension.clone();
                }
                OpenedDocument::Structure { document, .. } => {
                    if document.content().dimension != *dimension {
                        return Err(refuse("私有结构最终维度与实际内容不同"));
                    }
                }
            }
        }
        let mut sources = Vec::new();
        for (slot, dimension, revision) in std::mem::take(&mut self.sources) {
            if let Some(result) = opened.get(&slot) {
                let OpenedDocument::World { input, .. } = &result.baseline else {
                    return Err(refuse("新增结构夹带了世界读取来源"));
                };
                let session = input
                    .sessions
                    .get(&dimension)
                    .ok_or_else(|| refuse("新增文档读取来源没有实际维度"))?;
                if !Arc::ptr_eq(&revision, &session.revision()) {
                    return Err(refuse("新增文档读取来源已改变"));
                }
            } else {
                sources.push((slot, dimension, revision));
            }
        }
        self.sources = sources;
        let mut changes = Vec::new();
        for change in std::mem::take(&mut self.changes) {
            stop()?;
            if let Some(result) = opened.get_mut(&change.world) {
                let OpenedDocument::World { input, .. } = &mut result.document else {
                    return Err(refuse("新增结构夹带了世界修改证明"));
                };
                let session = input
                    .sessions
                    .get_mut(&change.dimension)
                    .ok_or_else(|| refuse("新增文档的修改维度没有实际会话"))?;
                if !Arc::ptr_eq(&change.baseline.revision(), &session.revision()) {
                    return Err(refuse("新增文档修改证明不属于实际加载基线"));
                }
                session.commit_snapshot_difference_cancellable(
                    change.prepared,
                    || cancelled.load(Ordering::Acquire),
                    &mut progress,
                )?;
            } else {
                changes.push(change);
            }
        }
        self.changes = changes;
        let mut structures = Vec::new();
        for output in std::mem::take(&mut self.structures) {
            stop()?;
            if let Some(result) = opened.get_mut(&output.slot) {
                let OpenedDocument::Structure { document, .. } = &result.baseline else {
                    return Err(refuse("新增世界夹带了结构修改结果"));
                };
                if !document.same_source(&output.baseline) {
                    return Err(refuse("新增结构修改结果不属于实际加载基线"));
                }
                result.document = OpenedDocument::Structure {
                    slot: output.slot,
                    document: output.document,
                };
            } else {
                structures.push(output);
            }
        }
        self.structures = structures;
        let mut selections = BTreeMap::new();
        for ((slot, dimension), selected) in std::mem::take(&mut self.selections) {
            if let Some(result) = opened.get_mut(&slot) {
                let target = result
                    .selections
                    .get_mut(&dimension)
                    .ok_or_else(|| refuse("新增文档选区不属于实际维度"))?;
                *target = selected;
                if let OpenedDocument::World { input, .. } = &mut result.document {
                    input.selections.insert(dimension, selected);
                }
            } else {
                selections.insert((slot, dimension), selected);
            }
        }
        self.selections = selections;
        if let Some(focus) = &self.focus {
            if let Some(result) = opened.get_mut(&focus.world) {
                if !result.selections.contains_key(&focus.dimension) {
                    return Err(refuse("新增文档视图不属于实际维度"));
                }
                if let OpenedDocument::World { input, .. } = &mut result.document {
                    input.dimension = focus.dimension.clone();
                }
                result.focus = self.focus.take();
            }
        }
        for slot in &self.save_order {
            if let Some(result) = opened.get_mut(slot) {
                result.save = true;
            }
        }
        self.save_order.retain(|slot| !opened.contains_key(slot));
        self.save_worlds.retain(|slot| !opened.contains_key(slot));
        self.save_sessions
            .retain(|(slot, _, _)| !opened.contains_key(slot));
        stop()?;
        Ok(ResolvedOutput {
            existing: self,
            opened: opened.into_values().collect(),
        })
    }
}
