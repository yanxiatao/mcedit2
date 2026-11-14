//! 连续工具尚未确认的新文档，以实际来源和稳定身份续接，不登记临时 GUI 会话。
use super::*;

#[derive(Debug, Clone)]
struct Entry {
    identity: String,
    output: script::OpenedOutput,
}
#[derive(Debug, Clone, Default)]
pub(crate) struct Candidates {
    entries: Vec<Entry>,
}
pub(crate) struct Absorbed {
    pub output: script::Output,
    pub slots: BTreeMap<usize, String>,
    pub save_order: Vec<usize>,
}
impl Candidates {
    pub(crate) fn documents(&self) -> impl Iterator<Item = (&str, &script::OpenedOutput)> {
        self.entries
            .iter()
            .map(|entry| (entry.identity.as_str(), &entry.output))
    }
    pub(crate) fn pending(&self) -> bool {
        !self.entries.is_empty()
    }
    pub(crate) fn summary(&self) -> String {
        self.entries
            .iter()
            .map(|entry| {
                format!(
                    "新文档 {}（确认后打开）",
                    entry.output.document.path().display()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub(crate) fn matches(&self, app: &App) -> bool {
        self.entries
            .iter()
            .all(|entry| match &entry.output.document {
                script::OpenedDocument::World { input, .. } => {
                    !app.worlds.iter().enumerate().any(|(index, world)| {
                        app.workspace.contains(index) && world.root == input.world.root
                    })
                }
                script::OpenedDocument::Structure { document, .. } => !app
                    .structures
                    .files
                    .values()
                    .any(|entry| entry.path == document.path()),
            })
    }
    pub(crate) fn work_bytes(&self) -> Result<usize, String> {
        self.entries.iter().try_fold(0usize, |bytes, entry| {
            bytes
                .checked_add(
                    entry
                        .output
                        .work_bytes()
                        .map_err(|error| error.to_string())?,
                )
                .and_then(|bytes| bytes.checked_add(entry.identity.len()))
                .ok_or("插件私有新文档合计工作集溢出".into())
        })
    }
    pub(crate) fn inputs(
        &self,
        slots: &BTreeMap<usize, String>,
    ) -> Result<Vec<script::OpenedOutput>, String> {
        self.entries
            .iter()
            .map(|entry| {
                let slot = slots
                    .iter()
                    .find_map(|(slot, identity)| (identity == &entry.identity).then_some(*slot))
                    .ok_or("私有新文档没有本回调的实际槽位")?;
                Ok(entry.output.clone().with_slot(slot))
            })
            .collect()
    }
    /// 调用方持有本回调的私有副本；任何错误会使整次回调结果不能交回正式候选。
    pub(crate) fn absorb(
        &mut self,
        output: script::Output,
        directory: &Directory,
        previous_slots: &BTreeMap<usize, String>,
        budget: EditWorkBudget,
        cancelled: &AtomicBool,
        progress: impl FnMut(mcedit2_world::editing::EditWorkProgress),
    ) -> Result<Absorbed, String> {
        let count = directory
            .tabs
            .iter()
            .filter(|tab| tab.document.is_some())
            .count();
        let identities = directory.identities(count)?;
        for (slot, identity) in previous_slots {
            if identities.get(*slot) != Some(identity) {
                return Err("私有新文档槽位与本回调的实际应用身份不同".into());
            }
        }
        let save_order = output.save_order.clone();
        let mut slots = previous_slots.clone();
        for document in &output.opened {
            let slot = document.slot();
            slots.insert(
                slot,
                identities
                    .get(slot)
                    .ok_or("新加载文档没有实际应用身份")?
                    .clone(),
            );
        }
        let private = self.inputs(previous_slots)?;
        let resolved = output
            .resolve_private(private, cancelled, progress)
            .map_err(|error| error.to_string())?;
        for following in resolved.opened {
            let identity = &identities[following.slot()];
            if let Some(previous) = self
                .entries
                .iter_mut()
                .find(|entry| &entry.identity == identity)
            {
                previous.output = previous
                    .output
                    .follow(following, budget, cancelled)
                    .map_err(|error| error.to_string())?;
            } else {
                self.entries.push(Entry {
                    identity: identity.clone(),
                    output: following,
                });
            }
        }
        Ok(Absorbed {
            output: resolved.existing,
            slots,
            save_order,
        })
    }
    pub(crate) fn prepare(
        self,
        budget: EditWorkBudget,
        cancelled: &AtomicBool,
    ) -> Result<Vec<(String, Ready)>, String> {
        self.entries
            .into_iter()
            .map(|entry| {
                Ok((
                    entry.identity,
                    Ready::read(entry.output, budget, cancelled)?,
                ))
            })
            .collect()
    }
}
