//! 整槽恢复的历史来源证明；精确恢复与基于原格式的后续编辑共用实际来源。
use super::*;

pub(super) struct EncodedSlot {
    pub method: u8,
    pub envelope: NbtEnvelope,
    pub bytes: Vec<u8>,
    pub external: bool,
}
pub(super) struct HistoricalTargets {
    pub targets: BTreeMap<Coord, Option<Vec<u8>>>,
    pub encodings: BTreeMap<Coord, EncodedSlot>,
}
pub(super) fn encoded_bytes(encodings: &BTreeMap<Coord, EncodedSlot>) -> Result<usize, WorldError> {
    encodings.values().try_fold(0usize, |used, slot| {
        used.checked_add(slot.bytes.len())
            .ok_or_else(|| invalid("历史槽位编码工作集溢出"))
    })
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Purpose {
    Exact,
    Restore { chunks: BTreeSet<Coord> },
    Edited { chunks: BTreeSet<Coord> },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchivedRegion {
    main: history_source::ArchivedFile,
    external: Vec<(Coord, history_source::ArchivedFile)>,
    purpose: Purpose,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    additional: Vec<ArchivedRegion>,
}
impl ArchivedRegion {
    pub(super) fn dependencies(&self, root: &std::path::Path) -> Result<Vec<PathBuf>, WorldError> {
        let mut paths = Vec::new();
        for file in std::iter::once(&self.main).chain(self.external.iter().map(|(_, file)| file)) {
            if let Some(path) = file.backup_path(root)? {
                paths.push(path);
            }
        }
        for part in &self.additional {
            paths.extend(part.dependencies(root)?);
        }
        Ok(paths)
    }
    pub(crate) fn new(
        main: history_source::ArchivedFile,
        external: Vec<(Coord, history_source::ArchivedFile)>,
    ) -> Self {
        Self {
            main,
            external,
            purpose: Purpose::Exact,
            additional: Vec::new(),
        }
    }
    fn exact(&self) -> bool {
        matches!(self.purpose, Purpose::Exact | Purpose::Restore { .. })
    }
    pub(super) fn covers(&self, chunk: Coord) -> bool {
        self.own_covers(chunk) || self.additional.iter().any(|part| part.own_covers(chunk))
    }
    fn own_covers(&self, chunk: Coord) -> bool {
        match &self.purpose {
            Purpose::Exact => true,
            Purpose::Edited { chunks } | Purpose::Restore { chunks } => chunks.contains(&chunk),
        }
    }
    fn part(&self, chunk: Coord) -> Option<&Self> {
        std::iter::once(self)
            .chain(&self.additional)
            .find(|part| part.own_covers(chunk))
    }
    pub(super) fn exact_chunk(&self, chunk: Coord) -> bool {
        self.part(chunk).is_some_and(Self::exact)
    }
    fn coverage(&self, written: &BTreeSet<Coord>) -> Result<BTreeSet<Coord>, WorldError> {
        match &self.purpose {
            Purpose::Exact => Ok(written.clone()),
            Purpose::Edited { chunks } | Purpose::Restore { chunks } => {
                if chunks.is_empty() || !chunks.is_subset(written) {
                    return Err(invalid("历史格式来源的范围为空或不属于写入槽"));
                }
                Ok(chunks.clone())
            }
        }
    }
    pub(super) fn validate(
        &self,
        storage: &Storage,
        chunks: &BTreeSet<Coord>,
    ) -> Result<(), WorldError> {
        if self.additional.len() >= chunks.len()
            || self
                .additional
                .iter()
                .any(|part| !part.additional.is_empty())
        {
            return Err(invalid("历史来源分组数量或层级无效"));
        }
        let mut selected = BTreeSet::new();
        for part in std::iter::once(self).chain(&self.additional) {
            part.validate_one(storage, chunks)?;
            for chunk in part.coverage(chunks)? {
                if !selected.insert(chunk) {
                    return Err(invalid("历史来源分组重复覆盖区块"));
                }
            }
        }
        Ok(())
    }
    fn validate_one(&self, storage: &Storage, chunks: &BTreeSet<Coord>) -> Result<(), WorldError> {
        if matches!(storage, Storage::Alpha { .. }) || self.external.len() > 1024 {
            return Err(invalid("Region 历史来源的存储类型或外部文件数量无效"));
        }
        self.main.validate()?;
        let scope = self.coverage(chunks)?;
        let mut seen = BTreeSet::new();
        for (chunk, file) in &self.external {
            if !scope.contains(chunk) || !seen.insert(*chunk) {
                return Err(invalid("Region 历史来源包含重复或无关 MCC"));
            }
            file.validate()?;
        }
        Ok(())
    }
    /// 返回是否仍有历史依赖；未改变的选择不会留在写入证明中。
    pub(super) fn retain(&mut self, chunks: &[ChunkIdentity]) -> bool {
        let mut additional = std::mem::take(&mut self.additional);
        additional.retain_mut(|part| part.retain_one(chunks));
        if self.retain_one(chunks) {
            self.additional = additional;
            return true;
        }
        if additional.is_empty() {
            return false;
        }
        let mut first = additional.remove(0);
        first.additional = additional;
        *self = first;
        true
    }
    fn retain_one(&mut self, chunks: &[ChunkIdentity]) -> bool {
        if let Purpose::Edited { chunks: scope } = &mut self.purpose {
            scope.retain(|chunk| {
                chunks
                    .iter()
                    .any(|entry| entry.chunk == *chunk && entry.nbt.is_some())
            });
        }
        if let Purpose::Restore { chunks: scope } = &mut self.purpose {
            scope.retain(|chunk| chunks.iter().any(|entry| entry.chunk == *chunk));
        }
        let keep = match &self.purpose {
            Purpose::Exact => None,
            Purpose::Edited { chunks } | Purpose::Restore { chunks } => Some(chunks),
        };
        self.external.retain(|(chunk, _)| {
            chunks
                .iter()
                .any(|entry| entry.chunk == *chunk && entry.nbt.is_some())
                && keep.is_none_or(|scope| scope.contains(chunk))
        });
        match &self.purpose {
            Purpose::Exact => true,
            Purpose::Edited { chunks } | Purpose::Restore { chunks } => !chunks.is_empty(),
        }
    }
    pub(super) fn require_exact_subset(&mut self) {
        if let Purpose::Edited { chunks } = &self.purpose {
            self.purpose = Purpose::Restore {
                chunks: chunks.clone(),
            };
        }
        for part in &mut self.additional {
            part.require_exact_subset();
        }
    }
    pub(super) fn grouped(mut parts: Vec<Self>) -> Result<Self, WorldError> {
        if parts.is_empty() {
            return Err(invalid("历史来源分组为空"));
        }
        if parts.iter().any(|part| !part.additional.is_empty()) {
            return Err(invalid("历史来源不能重复分组"));
        }
        let mut first = parts.remove(0);
        first.additional = parts;
        Ok(first)
    }
    pub(super) fn from_origin(
        world: &WorldSummary,
        dimension: &str,
        storage: &Storage,
        origin: &history_source::Archive,
        chunks: &BTreeSet<Coord>,
        retained: usize,
        budget: crate::editing::EditWorkBudget,
        cancelled: impl FnMut() -> bool,
    ) -> Result<(Self, HistoricalTargets), WorldError> {
        let path = storage_path(world, dimension, storage, &Role::Main)?;
        let (main, image) = origin.original_witness(&path, budget.bytes(), |bytes| {
            let mut used = retained;
            charge_budget(&mut used, bytes, budget)
        })?;
        let mut external = Vec::new();
        let loaded = read_image(
            world,
            dimension,
            storage,
            &image,
            chunks,
            retained,
            budget,
            &mut |chunk, reserve| {
                let path = storage_path(world, dimension, storage, &Role::External { chunk })?;
                let (proof, bytes) = origin.original_witness(&path, MAX_MCC, reserve)?;
                external.push((chunk, proof));
                Ok(bytes)
            },
            cancelled,
        )?;
        let history = Self {
            main,
            external,
            purpose: Purpose::Edited {
                chunks: chunks.clone(),
            },
            additional: Vec::new(),
        };
        history.validate(storage, chunks)?;
        Ok((history, loaded))
    }
    /// 明确文件版本的历史基底；旧原生事务的改写槽必须是内部编码。
    pub(super) fn from_version(
        world: &WorldSummary,
        dimension: &str,
        storage: &Storage,
        origin: &history_source::Archive,
        version: &history_source::FileVersion,
        chunks: &BTreeSet<Coord>,
        retained: usize,
        budget: crate::editing::EditWorkBudget,
        cancelled: impl FnMut() -> bool,
    ) -> Result<(Self, HistoricalTargets), WorldError> {
        let path = storage_path(world, dimension, storage, &Role::Main)?;
        if version.path() != path {
            return Err(invalid("明确历史版本不属于该 Region"));
        }
        let main = origin.archived_file(version)?;
        let image = origin
            .read_version_reserved(version, budget.bytes(), |bytes| {
                let mut used = retained;
                charge_budget(&mut used, bytes, budget)
            })?
            .ok_or_else(|| invalid("明确历史 Region 版本缺失"))?;
        let loaded = read_image(
            world,
            dimension,
            storage,
            &image,
            chunks,
            retained,
            budget,
            &mut |_, _| Err(invalid("该历史 Region 版本没有提供独立 MCC 来源证明")),
            cancelled,
        )?;
        let history = Self {
            main,
            external: Vec::new(),
            purpose: Purpose::Edited {
                chunks: chunks.clone(),
            },
            additional: Vec::new(),
        };
        history.validate(storage, chunks)?;
        Ok((history, loaded))
    }
    pub(super) fn targets(
        &self,
        world: &WorldSummary,
        dimension: &str,
        storage: &Storage,
        chunks: &BTreeSet<Coord>,
        retained: usize,
        budget: crate::editing::EditWorkBudget,
        source: &impl crate::entity_editing::IdentitySource,
        cancelled: impl FnMut() -> bool,
    ) -> Result<HistoricalTargets, WorldError> {
        self.validate(storage, chunks)?;
        let mut used = retained;
        let mut result = HistoricalTargets {
            targets: BTreeMap::new(),
            encodings: BTreeMap::new(),
        };
        let mut cancelled = cancelled;
        for part in std::iter::once(self).chain(&self.additional) {
            let loaded = part.targets_one(
                world,
                dimension,
                storage,
                chunks,
                used,
                budget,
                source,
                &mut cancelled,
            )?;
            charge_budget(&mut used, loaded.targets.len() * 128, budget)?;
            for raw in loaded.targets.values().flatten() {
                charge_budget(&mut used, raw.len(), budget)?;
            }
            charge_budget(&mut used, encoded_bytes(&loaded.encodings)?, budget)?;
            result.targets.extend(loaded.targets);
            result.encodings.extend(loaded.encodings);
        }
        Ok(result)
    }
    fn targets_one(
        &self,
        world: &WorldSummary,
        dimension: &str,
        storage: &Storage,
        chunks: &BTreeSet<Coord>,
        retained: usize,
        budget: crate::editing::EditWorkBudget,
        source: &impl crate::entity_editing::IdentitySource,
        cancelled: impl FnMut() -> bool,
    ) -> Result<HistoricalTargets, WorldError> {
        self.validate_one(storage, chunks)?;
        let scope = self.coverage(chunks)?;
        let path = storage_path(world, dimension, storage, &Role::Main)?;
        let image = self
            .main
            .read(&world.root, &path, budget.bytes(), source, |bytes| {
                let mut used = retained;
                charge_budget(&mut used, bytes, budget)
            })?;
        let mut used_external = BTreeSet::new();
        let loaded = read_image(
            world,
            dimension,
            storage,
            &image,
            &scope,
            retained,
            budget,
            &mut |chunk, reserve| {
                let file = self
                    .external
                    .iter()
                    .find(|(coordinate, _)| *coordinate == chunk)
                    .map(|(_, file)| file)
                    .ok_or_else(|| invalid("历史 Region 的外部槽没有对应 MCC 证明"))?;
                used_external.insert(chunk);
                let path = storage_path(world, dimension, storage, &Role::External { chunk })?;
                file.read(&world.root, &path, MAX_MCC, source, reserve)
            },
            cancelled,
        )?;
        if used_external.len() != self.external.len() {
            return Err(invalid("历史来源包含未关联的 MCC 证明"));
        }
        Ok(loaded)
    }
    pub(super) fn verify_loaded(
        &self,
        world: &WorldSummary,
        storage: &Storage,
        targets: &BTreeMap<Coord, Option<Vec<u8>>>,
        loaded: &HistoricalTargets,
    ) -> Result<(), WorldError> {
        for (chunk, before) in &loaded.targets {
            let part = self
                .part(*chunk)
                .ok_or_else(|| invalid("历史载荷没有对应的来源分组"))?;
            if part.exact() {
                if targets.get(chunk) != Some(before) {
                    return Err(invalid("恢复候选与实际历史槽位字节不同"));
                }
            } else {
                let before = before
                    .as_deref()
                    .ok_or_else(|| invalid("历史格式来源没有所声明的原槽"))?;
                let after = targets
                    .get(chunk)
                    .and_then(Option::as_deref)
                    .ok_or_else(|| invalid("历史格式证明未对应非空写入槽"))?;
                validate_payload(world, storage, *chunk, after, Some(before))?;
            }
        }
        Ok(())
    }
    pub(super) fn verify_targets(
        &self,
        world: &WorldSummary,
        proof: &Proof,
        targets: &BTreeMap<Coord, Option<Vec<u8>>>,
        retained: usize,
        source: &impl crate::entity_editing::IdentitySource,
        budget: crate::editing::EditWorkBudget,
    ) -> Result<BTreeMap<Coord, EncodedSlot>, WorldError> {
        let chunks = targets.keys().copied().collect();
        let loaded = self.targets(
            world,
            &proof.dimension,
            &proof.storage,
            &chunks,
            retained,
            budget,
            source,
            || false,
        )?;
        self.verify_loaded(world, &proof.storage, targets, &loaded)?;
        Ok(loaded.encodings)
    }
}
fn read_image(
    world: &WorldSummary,
    dimension: &str,
    storage: &Storage,
    image: &[u8],
    chunks: &BTreeSet<Coord>,
    retained: usize,
    budget: crate::editing::EditWorkBudget,
    external: &mut impl FnMut(
        Coord,
        &mut dyn FnMut(usize) -> Result<(), WorldError>,
    ) -> Result<Vec<u8>, WorldError>,
    mut cancelled: impl FnMut() -> bool,
) -> Result<HistoricalTargets, WorldError> {
    if chunks.is_empty() || chunks.len() > 1024 {
        return Err(invalid("历史 Region 的选择数量无效"));
    }
    let region = match storage {
        Storage::Terrain { region, .. }
        | Storage::Entities { region }
        | Storage::Poi { region } => *region,
        Storage::Alpha { .. } => return Err(invalid("Alpha 不能使用 Region 历史来源")),
    };
    let mut used = retained;
    charge_budget(&mut used, image.len(), budget)?;
    charge_budget(&mut used, chunks.len() * 128, budget)?;
    let source_slots = slots(Some(image))?;
    let mut targets = BTreeMap::new();
    let mut encodings = BTreeMap::new();
    for &chunk in chunks {
        if cancelled() {
            return Err(invalid("历史槽位读取已取消"));
        }
        if (chunk.0.div_euclid(32), chunk.1.div_euclid(32)) != region {
            return Err(invalid("历史槽位越过声明的 Region"));
        }
        let raw = match source_slots[index(chunk)] {
            None => None,
            Some(slot) => {
                let external_method = crate::region_external::is_external(image, slot)?;
                let (method, envelope, bytes, raw) = if let Some(method) = external_method {
                    let bytes =
                        external(chunk, &mut |bytes| charge_budget(&mut used, bytes, budget))?;
                    let (envelope, raw) = crate::region_external::decode(method, &bytes)?;
                    (method, envelope, bytes, raw)
                } else {
                    let (method, envelope, raw) = region_plan::original_nbt(image, slot)?;
                    let offset = slot.start * SECTOR;
                    let length = u32::from_be_bytes(
                        image[offset..offset + 4]
                            .try_into()
                            .expect("槽长度已由 Region 解析器核对"),
                    ) as usize;
                    charge_budget(&mut used, length - 1, budget)?;
                    (
                        method,
                        envelope,
                        image[offset + 5..offset + 4 + length].to_vec(),
                        raw,
                    )
                };
                charge_budget(&mut used, raw.len(), budget)?;
                validate_payload(world, storage, chunk, &raw, Some(&raw))?;
                encodings.insert(
                    chunk,
                    EncodedSlot {
                        method,
                        envelope,
                        bytes,
                        external: external_method.is_some(),
                    },
                );
                Some(raw)
            }
        };
        targets.insert(chunk, raw);
    }
    Ok(HistoricalTargets { targets, encodings })
}
