//! 整组选区统一分配 UUID，证明只保存所有权和实际引用边，避免逐根复制完整映射。
use super::*;

struct Root {
    compound_sha: [u8; 32],
    temporary: Vec<(String, [u8; 16])>,
}
pub(crate) struct ImportPlan {
    roots: Vec<Root>,
    replacements: BTreeMap<[u8; 16], [u8; 16]>,
    reference_scope: [u8; 32],
    positions: crate::entity_references::Locations,
    delta: [i32; 3],
    data_version: i32,
}

/// 最终导入分配中只属于新增节点的身份和位置；已有目标身份仍指向原对象。
#[derive(Debug, Clone, Default)]
pub(crate) struct ReferenceRemap {
    ids: BTreeMap<[u8; 16], [u8; 16]>,
    positions: crate::entity_references::Locations,
}
impl ReferenceRemap {
    pub(crate) fn work_bytes(&self) -> usize {
        (self.ids.len() + self.positions.len()) * 128
    }

    /// 复用版本化的引用字段与搜索坐标适配，不更改现存实体树自身 UUID 或普通坐标。
    pub(crate) fn rewrite(
        &self,
        compound: &[u8],
        data_version: Option<i32>,
        budget: crate::editing::EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Vec<u8>>, WorldError> {
        if cancelled() {
            return Err(invalid("脚本新增实体引用绑定已取消"));
        }
        if self.ids.is_empty() {
            return Ok(None);
        }
        budget.check(
            compound
                .len()
                .checked_mul(8)
                .and_then(|bytes| bytes.checked_add(self.work_bytes()))
                .ok_or_else(|| invalid("实体关联改写工作集溢出"))?,
        )?;
        let mut raw = vec![10, 0, 0];
        raw.extend_from_slice(compound);
        let node = parse_root(&raw).map_err(|error| invalid(error.to_string()))?;
        let mut operations = Vec::new();
        let mut diagnostics = Vec::new();
        crate::entity_references::relocate_tree(
            &node,
            "",
            data_version,
            &mut |id| self.positions.get(&id).copied(),
            &mut operations,
        )?;
        entity_tree_operations(
            &node,
            "",
            None,
            &mut |id| self.ids.get(&id).copied(),
            data_version,
            &mut operations,
            &mut diagnostics,
        )?;
        for diagnostic in diagnostics {
            log::warn!("[script-entity-reference] {diagnostic}");
        }
        if cancelled() {
            return Err(invalid("脚本新增实体引用绑定已取消"));
        }
        if operations.is_empty() {
            return Ok(None);
        }
        Ok(Some(patch_batch(&raw, &operations)?[3..].to_vec()))
    }
}

pub(super) fn replacement_digest(map: &BTreeMap<[u8; 16], [u8; 16]>) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"mcedit2.clone-reference-scope.v1");
    for (old, next) in map {
        digest.update(old);
        digest.update(next);
    }
    digest.finalize().into()
}

/// 所有根和乘客先统一收集身份，再分配缺失节点与最终 UUID；重复源身份直接报错。
pub(crate) fn plan_imports(
    entities: &[crate::clipboard::OpaqueEntity],
    data_version: Option<i32>,
    delta: [i32; 3],
    snapshot: &IdentitySnapshot,
    budget: crate::editing::EditWorkBudget,
    cancelled: impl FnMut() -> bool,
) -> Result<ImportPlan, WorldError> {
    plan_imports_with(
        entities,
        data_version,
        delta,
        budget,
        cancelled,
        |count, excluded| snapshot.fresh_ids_excluding(count, excluded),
    )
}

fn plan_imports_with(
    entities: &[crate::clipboard::OpaqueEntity],
    data_version: Option<i32>,
    delta: [i32; 3],
    budget: crate::editing::EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
    mut allocate: impl FnMut(usize, &BTreeSet<[u8; 16]>) -> Result<Vec<[u8; 16]>, WorldError>,
) -> Result<ImportPlan, WorldError> {
    let data_version = data_version
        .filter(|v| matches!(*v, 1343 | 3465 | 3955))
        .ok_or_else(|| invalid("整组实体导入的数据版本尚未核实"))?;
    let mut ids = BTreeSet::new();
    let mut roots = Vec::with_capacity(entities.len());
    let mut missing = Vec::with_capacity(entities.len());
    let mut count = 0usize;
    let mut path_bytes = 0usize;
    for entity in entities {
        if cancelled() {
            return Err(invalid("整组实体身份分配已取消"));
        }
        let mut raw = vec![10, 0, 0];
        raw.extend_from_slice(&entity.compound);
        let node = parse_root(&raw).map_err(|e| invalid(e.to_string()))?;
        let mut paths = Vec::new();
        missing_uuid_paths(&node, "", &mut paths, &mut ids)?;
        count = count
            .checked_add(paths.len())
            .ok_or_else(|| invalid("整组临时身份数量溢出"))?;
        for path in &paths {
            path_bytes = path_bytes
                .checked_add(path.len() + std::mem::size_of::<(String, [u8; 16])>())
                .ok_or_else(|| invalid("整组身份路径工作集溢出"))?;
        }
        let work = ids
            .len()
            .checked_add(count)
            .and_then(|n| n.checked_mul(384))
            .and_then(|n| n.checked_add(path_bytes))
            .and_then(|n| {
                entities
                    .len()
                    .checked_mul(std::mem::size_of::<Root>())
                    .and_then(|roots| n.checked_add(roots))
            })
            .ok_or_else(|| invalid("整组身份工作集溢出"))?;
        budget.check(work)?;
        roots.push(Root {
            compound_sha: Sha256::digest(&entity.compound).into(),
            temporary: Vec::new(),
        });
        missing.push(paths);
    }
    let temporary = allocate(count, &ids)?;
    ids.extend(temporary.iter().copied());
    let mut temporary = temporary.into_iter();
    for (root, paths) in roots.iter_mut().zip(missing) {
        root.temporary = paths
            .into_iter()
            .map(|path| {
                (
                    path,
                    temporary.next().expect("临时 UUID 数量已按全部路径分配"),
                )
            })
            .collect();
    }
    if cancelled() {
        return Err(invalid("整组实体身份分配已取消"));
    }
    let mut positions = BTreeMap::new();
    for (root, entity) in roots.iter().zip(entities) {
        if cancelled() {
            return Err(invalid("整组关联位置收集已取消"));
        }
        let raw = complete_import_nbt(&entity.compound, &root.temporary, data_version)?;
        let node = parse_root(&raw).map_err(|e| invalid(e.to_string()))?;
        for (id, point) in crate::entity_references::locations(&node, delta)? {
            if positions.insert(id, point).is_some() {
                return Err(invalid("整组关联位置包含重复 UUID"));
            }
        }
    }
    let fresh = allocate(ids.len(), &ids)?;
    let replacements = ids.into_iter().zip(fresh).collect::<BTreeMap<_, _>>();
    let reference_scope = replacement_digest(&replacements);
    Ok(ImportPlan {
        roots,
        replacements,
        reference_scope,
        positions,
        delta,
        data_version,
    })
}

/// 独立文档不构造世界身份快照；直接以真实目标实体树分配身份和重映射整组引用。
pub(crate) fn clone_compounds(
    entities: &[crate::clipboard::OpaqueEntity],
    target: &[crate::clipboard::OpaqueEntity],
    data_version: Option<i32>,
    delta: [i32; 3],
    dimensions: &DimensionChange,
    budget: crate::editing::EditWorkBudget,
    cancelled: impl FnMut() -> bool,
) -> Result<Vec<Vec<u8>>, WorldError> {
    Ok(clone_compounds_linked(
        entities,
        target,
        data_version,
        delta,
        dimensions,
        budget,
        cancelled,
    )?
    .0)
}

/// 脚本同时修改既有节点时，返回同一次分配的关联表，避免临时 UUID 留在既有引用中。
pub(crate) fn clone_compounds_linked(
    entities: &[crate::clipboard::OpaqueEntity],
    target: &[crate::clipboard::OpaqueEntity],
    data_version: Option<i32>,
    delta: [i32; 3],
    dimensions: &DimensionChange,
    budget: crate::editing::EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> Result<(Vec<Vec<u8>>, ReferenceRemap), WorldError> {
    let mut occupied = BTreeSet::new();
    for entity in target {
        if cancelled() {
            return Err(invalid("目标结构身份读取已取消"));
        }
        let mut raw = vec![10, 0, 0];
        raw.extend_from_slice(&entity.compound);
        let node = parse_root(&raw).map_err(|error| invalid(error.to_string()))?;
        let mut missing = Vec::new();
        missing_uuid_paths(&node, "", &mut missing, &mut occupied)?;
    }
    let plan = plan_imports_with(
        entities,
        data_version,
        delta,
        budget,
        &mut cancelled,
        |count, excluded| fresh_ids_excluding(count, &occupied, excluded),
    )?;
    let references = plan.existing_references(|id| occupied.contains(id));
    let compounds = entities
        .iter()
        .zip(&plan.roots)
        .map(|(entity, root)| {
            if cancelled() {
                return Err(invalid("结构实体复制已取消"));
            }
            let raw = complete_import_nbt(&entity.compound, &root.temporary, plan.data_version)?;
            let moved =
                offset_compound(&raw[3..], delta, data_version, dimensions, &plan.positions)?;
            let mut named = vec![10, 0, 0];
            named.extend_from_slice(&moved);
            let node = parse_root(&named).map_err(|error| invalid(error.to_string()))?;
            let mut operations = Vec::new();
            let mut diagnostics = Vec::new();
            clone_operations(
                &node,
                "",
                &plan.replacements,
                &mut |id| plan.replacements.get(&id).copied(),
                data_version,
                &mut operations,
                &mut diagnostics,
            )?;
            for diagnostic in diagnostics {
                log::warn!("[structure-entity-copy] {diagnostic}");
            }
            Ok(patch_batch(&named, &operations)?[3..].to_vec())
        })
        .collect::<Result<Vec<_>, WorldError>>()?;
    Ok((compounds, references))
}

impl ImportPlan {
    /// 原存档中已经存在的身份不改指克隆对象；只绑定来源为本批新增树的身份。
    pub(crate) fn existing_references(
        &self,
        occupied: impl Fn(&[u8; 16]) -> bool,
    ) -> ReferenceRemap {
        let ids = self
            .replacements
            .iter()
            .filter(|(id, _)| !occupied(id))
            .map(|(&id, &next)| (id, next))
            .collect::<BTreeMap<_, _>>();
        let positions = self
            .positions
            .iter()
            .filter(|(id, _)| ids.contains_key(*id))
            .map(|(&id, &position)| (id, position))
            .collect();
        ReferenceRemap { ids, positions }
    }
    fn root(&self, index: usize) -> Result<&Root, WorldError> {
        self.roots
            .get(index)
            .ok_or_else(|| invalid("整组实体来源索引越界"))
    }
    pub(crate) fn clone_entity(
        &self,
        index: usize,
        source: &[u8],
        target: Option<&[u8]>,
        source_chunk: (i32, i32),
        target_chunk: (i32, i32),
        source_separate: bool,
        target_separate: bool,
        source_index: usize,
        delta: [i32; 3],
        snapshot: &IdentitySnapshot,
        dimensions: DimensionChange,
    ) -> Result<CloneEntity, WorldError> {
        let root = self.root(index)?;
        if delta != self.delta {
            return Err(invalid("整组克隆位移与关联位置计划不同"));
        }
        let raw = crate::nbt_document::raw_compound_list_item(
            source,
            list_path(source_separate, source)?,
            source_index,
        )?;
        if !root.temporary.is_empty() || <[u8; 32]>::from(Sha256::digest(raw)) != root.compound_sha
        {
            return Err(invalid("整组克隆的原始实体与分配计划不同"));
        }
        clone_entity_inner(
            source,
            target,
            source_chunk,
            target_chunk,
            source_separate,
            target_separate,
            source_index,
            delta,
            snapshot,
            false,
            dimensions,
            CloneAllocation::Planned(&self.replacements, &self.positions, self.reference_scope),
        )
    }
    pub(crate) fn import_entity(
        &self,
        index: usize,
        compound: &[u8],
        target: Option<&[u8]>,
        target_chunk: (i32, i32),
        target_separate: bool,
        delta: [i32; 3],
        snapshot: &IdentitySnapshot,
        dimensions: DimensionChange,
    ) -> Result<ImportEntity, WorldError> {
        let root = self.root(index)?;
        if delta != self.delta {
            return Err(invalid("整组导入位移与关联位置计划不同"));
        }
        import_entity_allocated(
            compound,
            target,
            target_chunk,
            target_separate,
            delta,
            Some(self.data_version),
            snapshot,
            dimensions,
            ImportAllocation::Planned {
                compound_sha: root.compound_sha,
                temporary: &root.temporary,
                replacements: &self.replacements,
                positions: &self.positions,
                reference_scope: self.reference_scope,
            },
        )
    }
}

/// 按整组摘要合并所有权，核对每一条真实引用；缺少任意根证明都不能独立提交或恢复。
pub(crate) fn verify_clone_scopes<'a>(
    proofs: impl IntoIterator<Item = &'a CloneEntity>,
) -> Result<(), WorldError> {
    let mut scopes = BTreeMap::<
        [u8; 32],
        (
            BTreeMap<[u8; 16], [u8; 16]>,
            crate::entity_references::Locations,
            Vec<&CloneEntity>,
        ),
    >::new();
    for proof in proofs {
        let (map, positions, members) = scopes.entry(proof.reference_scope).or_default();
        if !proof.owned_positions.keys().eq(proof.replacements.keys()) {
            return Err(invalid("克隆关联位置与身份所有权不完整"));
        }
        for (&old, &next) in &proof.replacements {
            if map.insert(old, next).is_some() {
                return Err(invalid("整组克隆重复拥有同一源 UUID"));
            }
        }
        positions.extend(
            proof
                .owned_positions
                .iter()
                .map(|(id, position)| (*id, *position)),
        );
        members.push(proof);
    }
    for (scope, (map, positions, members)) in scopes {
        if replacement_digest(&map) != scope {
            return Err(invalid("整组克隆缺少实体身份所有权证明"));
        }
        let mut fresh = BTreeSet::new();
        for next in map.values() {
            if map.contains_key(next) || !fresh.insert(*next) {
                return Err(invalid("整组克隆的新 UUID 重复或与源身份冲突"));
            }
        }
        for proof in members {
            let expected_positions = proof
                .locator_sources
                .iter()
                .filter_map(|id| positions.get(id).map(|position| (*id, *position)))
                .collect::<BTreeMap<_, _>>();
            if expected_positions != proof.locator_positions {
                return Err(invalid("整组关联搜索坐标未指向目标实体的实际位置"));
            }
            let expected = proof
                .source_references
                .iter()
                .filter_map(|id| map.get(id).map(|next| (*id, *next)))
                .collect::<BTreeMap<_, _>>();
            if expected != proof.reference_replacements {
                return Err(invalid("整组克隆的实体引用未按完整所有权映射重建"));
            }
        }
    }
    Ok(())
}
