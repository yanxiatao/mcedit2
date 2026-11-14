//! 原版整区块操作的冻结计划；只生成真实存储候选，写入沿用混合事务。
use super::*;
use crate::{
    editing::{EditWorkBudget, EditWorkProgress},
    mixed_transaction::MixedCandidate,
    RegionIndex,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    DeleteSelected,
    PruneOutside,
    MarkRepopulation,
    UnmarkRepopulation,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::DeleteSelected => "删除选中区块",
            Self::PruneOutside => "保留选区并修剪维度",
            Self::MarkRepopulation => "标记为需要重新填充",
            Self::UnmarkRepopulation => "取消重新填充标记",
        }
    }
    pub fn deletes(self) -> bool {
        matches!(self, Self::DeleteSelected | Self::PruneOutside)
    }
    pub fn phrase(self) -> &'static str {
        if self.deletes() {
            "DELETE_CHUNKS"
        } else {
            "SAVE"
        }
    }
}
impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}
/// 包含边界的区块坐标矩形；按原版 chunkPositions，只使用选框的 XZ 覆盖。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub minimum: [i32; 2],
    pub maximum: [i32; 2],
}
impl Rect {
    pub fn validate(self) -> Result<(), WorldError> {
        if (0..2).any(|i| self.minimum[i] > self.maximum[i]) {
            Err(err("区块选择边界无效"))
        } else {
            Ok(())
        }
    }
    pub fn contains(self, (x, z): (i32, i32)) -> bool {
        x >= self.minimum[0] && x <= self.maximum[0] && z >= self.minimum[1] && z <= self.maximum[1]
    }
}
#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: PathBuf,
    pub storage: Storage,
    pub after_bytes: Option<usize>,
}
#[derive(Debug)]
pub struct Plan {
    pub root: PathBuf,
    pub dimension: String,
    pub kind: Kind,
    pub scope: Rect,
    pub timestamp: u32,
    pub chunks: Vec<(i32, i32)>,
    pub files: Vec<FileChange>,
    pub unchanged: usize,
    candidates: Vec<MixedCandidate>,
    bytes: usize,
}
impl Plan {
    pub fn candidates(&self) -> &[MixedCandidate] {
        &self.candidates
    }
    pub fn work_bytes(&self) -> usize {
        self.bytes
    }
    pub fn summary(&self) -> String {
        format!("{} · {}\n区块范围 {:?} .. {:?}；涉及 {} 个实际区块、{} 个文件；{} 个区块的标志已是目标值。此处为冻结计划，执行状态以实际事务记录为准。",self.kind.label(),self.dimension,self.scope.minimum,self.scope.maximum,self.chunks.len(),self.files.len(),self.unchanged)
    }
}
fn extension(world: &WorldSummary, dimension: &str) -> Result<String, WorldError> {
    let dim = world
        .dimensions
        .iter()
        .find(|d| d.name == dimension)
        .ok_or_else(|| err("维度不存在"))?;
    let formats: Vec<_> = dim
        .extension
        .split(", ")
        .filter(|v| matches!(*v, "mca" | "mcr"))
        .collect();
    match formats.as_slice() {
        [extension] => Ok((*extension).to_owned()),
        [] => Err(err("整区块操作需要实际 MCA/MCR 存储")),
        _ => Err(err("维度同时含 MCA 和 MCR，无法按单一存储合同规划")),
    }
}
/// 扫描真实三个存储域；不枚举选框中的缺失槽，不从地图范围推断已有区块。
pub fn prepare(
    world: &WorldSummary,
    dimension: &str,
    kind: Kind,
    scope: Rect,
    timestamp: u32,
    budget: EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
    mut progress: impl FnMut(EditWorkProgress),
) -> Result<Plan, WorldError> {
    scope.validate()?;
    stop(&mut cancelled)?;
    let ext = extension(world, dimension)?;
    let base = dimension_root(world, dimension)?;
    let mut inventories = Vec::new();
    let mut slots = BTreeMap::<(i32, i32), BTreeSet<Storage>>::new();
    let mut visited = 0usize;
    let mut retained = 0usize;
    for storage in [Storage::Terrain, Storage::Entities, Storage::Poi] {
        stop(&mut cancelled)?;
        let dir = base.join(storage.directory());
        let inventory = inventory_cancellable(&dir, &ext, budget, &mut cancelled)?;
        let alternative = if ext == "mca" { "mcr" } else { "mca" };
        if !inventory_cancellable(&dir, alternative, budget, &mut cancelled)?.is_empty() {
            return Err(err("存储目录含另一种 Region 格式，无法核对完整区块清单"));
        }
        retained += inventory
            .iter()
            .map(|(name, sha)| name.len() + sha.len() + 96)
            .sum::<usize>();
        for (name, _) in &inventory {
            stop(&mut cancelled)?;
            let parts: Vec<_> = name.split('.').collect();
            let rx = parts[1].parse::<i32>().map_err(|_| err("Region X 无效"))?;
            let rz = parts[2].parse::<i32>().map_err(|_| err("Region Z 无效"))?;
            let index = RegionIndex::open(dir.join(name))?;
            for z in 0..32u8 {
                for x in 0..32u8 {
                    if index.slot(x, z).is_none() {
                        continue;
                    }
                    let cx = i32::try_from(i64::from(rx) * 32 + i64::from(x))
                        .map_err(|_| err("区块 X 超出范围"))?;
                    let cz = i32::try_from(i64::from(rz) * 32 + i64::from(z))
                        .map_err(|_| err("区块 Z 超出范围"))?;
                    slots.entry((cx, cz)).or_default().insert(storage);
                }
            }
            visited += 1;
            budget.check(retained.saturating_add(slots.len().saturating_mul(160)))?;
            progress(EditWorkProgress {
                stage: "扫描实际区块存储",
                examined: visited,
                selected: slots.len(),
                slices: visited,
                ..Default::default()
            });
        }
        inventories.push((storage, inventory));
    }
    stop(&mut cancelled)?;
    let selected: Vec<_> = slots
        .iter()
        .filter(|(position, _)| {
            if kind == Kind::PruneOutside {
                !scope.contains(**position)
            } else {
                scope.contains(**position)
            }
        })
        .map(|(&position, _)| position)
        .collect();
    if selected.is_empty() {
        return Err(err("所选范围没有符合操作的实际存储槽"));
    }
    let mut candidates = Vec::new();
    let mut chunks = Vec::new();
    let mut unchanged = 0;
    let mut files = Vec::new();
    if kind.deletes() {
        let mut deleted = prepare_delete_cancellable(
            world,
            dimension,
            &selected,
            timestamp,
            budget,
            &mut cancelled,
            &mut progress,
        )?;
        if kind == Kind::PruneOutside {
            deleted.proof.storage_inventories = Some(inventories);
        }
        for region in &deleted.regions {
            files.push(FileChange {
                path: region.path.clone(),
                storage: region.identity.storage,
                after_bytes: Some(region.image.len()),
            });
        }
        for side in &deleted.sidecars {
            files.push(FileChange {
                path: side.path.clone(),
                storage: side.identity.storage,
                after_bytes: None,
            });
        }
        chunks = selected;
        candidates.push(MixedCandidate::DeleteChunks(deleted));
    } else {
        let populated = kind == Kind::UnmarkRepopulation;
        let mut groups = BTreeMap::<(i32, i32), Vec<(i32, i32)>>::new();
        for position in selected {
            if slots[&position].contains(&Storage::Terrain) {
                groups
                    .entry((position.0.div_euclid(32), position.1.div_euclid(32)))
                    .or_default()
                    .push(position);
            }
        }
        let mut used = retained + slots.len() * 160;
        for ((rx, rz), positions) in groups {
            stop(&mut cancelled)?;
            let path = base.join("region").join(format!("r.{rx}.{rz}.{ext}"));
            let index = RegionIndex::open(&path)?;
            let mut changed = Vec::new();
            for position in positions {
                stop(&mut cancelled)?;
                let raw = index
                    .read_chunk_at(position.0, position.1)?
                    .ok_or_else(|| err("实际地形槽在扫描期间消失"))?;
                budget.check(used.saturating_add(raw.len().saturating_mul(4)))?;
                if populated_value(&raw)? == populated {
                    unchanged += 1;
                } else {
                    changed.push(position);
                }
            }
            if changed.is_empty() {
                continue;
            }
            let available = budget
                .bytes()
                .checked_sub(used)
                .filter(|&n| n > 0)
                .ok_or_else(|| err("区块标记计划超出任务预算"))?;
            let candidate = prepare_repopulation_cancellable(
                world,
                dimension,
                (rx, rz),
                &ext,
                &changed,
                populated,
                timestamp,
                EditWorkBudget::from_bytes(available)?,
                &mut cancelled,
            )?;
            used += candidate.image.len();
            budget.check(used)?;
            files.push(FileChange {
                path: candidate.path.clone(),
                storage: Storage::Terrain,
                after_bytes: Some(candidate.image.len()),
            });
            chunks.extend(changed);
            candidates.push(MixedCandidate::Repopulate(candidate));
            progress(EditWorkProgress {
                stage: "准备旧版重新填充标志",
                examined: files.len(),
                selected: chunks.len() + unchanged,
                changed: chunks.len(),
                slices: files.len(),
            });
        }
        if candidates.is_empty() {
            return Err(err("所选地形没有需要改变的旧版重新填充标志"));
        }
    }
    chunks.sort_unstable();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut bytes = chunks.len() * 16
        + files
            .iter()
            .map(|f| std::mem::size_of::<FileChange>() + f.path.as_os_str().len() * 2)
            .sum::<usize>();
    for candidate in &candidates {
        match candidate {
            MixedCandidate::DeleteChunks(c) => {
                bytes += c.regions.iter().map(|r| r.image.len()).sum::<usize>()
                    + serde_json::to_vec(&c.proof)
                        .map_err(|e| err(e.to_string()))?
                        .len();
            }
            MixedCandidate::Repopulate(c) => {
                bytes += c.image.len()
                    + serde_json::to_vec(&c.proof)
                        .map_err(|e| err(e.to_string()))?
                        .len();
            }
            _ => unreachable!("计划只构造实际区块生命周期候选"),
        }
    }
    budget.check(bytes.saturating_mul(2))?;
    stop(&mut cancelled)?;
    Ok(Plan {
        root: world.root.clone(),
        dimension: dimension.into(),
        kind,
        scope,
        timestamp,
        chunks,
        files,
        unchanged,
        candidates,
        bytes,
    })
}
