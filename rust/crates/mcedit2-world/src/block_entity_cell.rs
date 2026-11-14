//! 单格检查及方块实体生命周期；只修改真实载荷，保留方块、实体、群系和未知字段。
use super::*;
#[derive(Debug, Clone)]
pub struct Body {
    pub id: Option<String>,
    pub sha256: String,
    pub bytes: usize,
}
pub(crate) fn body(compound: &[u8]) -> Result<Body, WorldError> {
    let mut raw = vec![10, 0, 0];
    raw.extend_from_slice(compound);
    let root = crate::sections::parse_root(&raw).map_err(refuse)?;
    let id = root
        .field("id")
        .map(|id| id.string().map_err(refuse))
        .transpose()?;
    Ok(Body {
        id,
        sha256: nbt_sha(compound),
        bytes: compound.len(),
    })
}
#[derive(Debug, Clone)]
pub struct Cell {
    pub position: [i32; 3],
    pub state: Option<State>,
    pub version: Option<i32>,
    pub light: Option<crate::saved_light::Levels>,
    pub light_ready: Option<bool>,
    pub description: String,
    pub block_entity: Option<Body>,
    pub(crate) source_identity: Option<String>,
}
impl Cell {
    pub fn same_contents(&self, other: &Self) -> bool {
        self.position == other.position
            && self.state == other.state
            && self.version == other.version
            && self.source_identity == other.source_identity
            && self.block_entity.as_ref().map(|body| &body.sha256)
                == other.block_entity.as_ref().map(|body| &body.sha256)
    }
}
#[derive(Debug, Clone)]
pub enum Mutation {
    Create {
        id: String,
        payload: Option<crate::nbt_document::Value>,
    },
    Remove,
}
#[derive(Debug, Clone)]
pub struct Options {
    pub expected: Cell,
    pub mutation: Mutation,
}
#[derive(Debug, Clone)]
pub struct Prepared {
    group: PreparedStructure,
    pub position: [i32; 3],
    pub added: bool,
    pub bytes: usize,
}
impl Prepared {
    pub fn summary(&self) -> String {
        format!(
            "{}方块实体：位置 {:?}，{} 字节原生载荷。原方块状态保留；一次撤销，尚未保存。",
            if self.added { "创建" } else { "移除" },
            self.position,
            self.bytes
        )
    }
    pub fn commit(
        self,
        session: &mut Session,
        cancelled: impl FnMut() -> bool,
        progress: impl FnMut(EditWorkProgress),
    ) -> Result<usize, WorldError> {
        session
            .commit_structure_with_progress_cancellable(self.group, cancelled, progress)
            .map(|_| 1)
    }
}
impl PreparedStructure {
    pub(super) fn payload_transaction(session: &Session, budget: EditWorkBudget) -> Self {
        Self {
            work_budget: budget,
            root: session.world.root.clone(),
            dimension: session.dimension.clone(),
            identity_revision: None,
            target_sources: BTreeMap::new(),
            creation_before: BTreeMap::new(),
            creation_after: BTreeMap::new(),
            source_block_entities: Vec::new(),
            source_tick_sha: BTreeMap::new(),
            source_entities: Vec::new(),
            source_world: None,
            source_dimension: session.dimension.clone(),
            source_blocks: Vec::new(),
            source_terrain_sha: BTreeMap::new(),
            source_creation: BTreeMap::new(),
            source_file: None,
            source_biomes: Vec::new(),
            source_numeric_biomes: Vec::new(),
            blocks: Vec::new(),
            be_chunks: Vec::new(),
            be_proofs: Vec::new(),
            tick_proofs: Vec::new(),
            biome_chunks: Vec::new(),
            biome_proofs: Vec::new(),
            entity_groups: Vec::new(),
            script_references: crate::entity_editing::ReferenceRemap::default(),
            source_metadata: None,
            diagnostics: Vec::new(),
            overwritten_block_entities: 0,
            overwritten_inventory_stacks: 0,
        }
    }
}
fn stop(cancelled: &mut impl FnMut() -> bool) -> Result<(), WorldError> {
    if cancelled() {
        Err(refuse("方块实体操作已取消"))
    } else {
        Ok(())
    }
}
impl Session {
    pub fn inspect_block_entity_cell_cancellable(
        &self,
        position: [i32; 3],
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Cell, WorldError> {
        stop(&mut cancelled)?;
        Bounds::spatial(position.into(), position.into())?;
        let (x, y, z) = (position[0], position[1], position[2]);
        let lookup = self.visible_block((x, y, z))?;
        let missing = matches!(&lookup, crate::block_lookup::Lookup::NoChunk);
        let description = lookup.description();
        let (state, version, light, light_ready) = match lookup {
            crate::block_lookup::Lookup::Present(block) => (
                Some(block.state),
                block.original_data_version,
                Some(block.light),
                block.light_ready,
            ),
            _ => (None, None, None, None),
        };
        let mut cell = Cell {
            position,
            state,
            version,
            light,
            light_ready,
            description,
            block_entity: None,
            source_identity: None,
        };
        if missing {
            return Ok(cell);
        }
        let chunk = (x.div_euclid(16), z.div_euclid(16));
        let snapshot = self.creation.get(&chunk);
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let input = structure_payload_input(
            &mut reader,
            chunk,
            snapshot,
            snapshot,
            self.be_overlay.get(&chunk),
            self.be_source.get(&chunk),
        )?;
        budget.check(
            input
                .before
                .len()
                .checked_mul(8)
                .ok_or_else(|| refuse("单格检查工作集溢出"))?,
        )?;
        let decoded = decode_sections(&input.before).map_err(|e| refuse(e.to_string()))?;
        cell.version = decoded.data_version;
        cell.source_identity = Some(input.source_sha);
        cell.block_entity =
            crate::sections::inspect_original_block_entity(&input.before, (x, y, z))
                .map_err(|e| refuse(e.to_string()))?
                .map(|body| Body {
                    id: body.raw_id,
                    sha256: body.original_compound_sha256,
                    bytes: body.original_compound_bytes,
                });
        stop(&mut cancelled)?;
        Ok(cell)
    }
    pub fn prepare_block_entity_cell_cancellable(
        &self,
        options: Options,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Prepared, WorldError> {
        self.ensure_history_slot(false)?;
        let cell = self.inspect_block_entity_cell_cancellable(
            options.expected.position,
            budget,
            &mut cancelled,
        )?;
        if !cell.same_contents(&options.expected) {
            return Err(refuse("所选方块或方块实体来源已改变，请重新检查"));
        }
        let position = (cell.position[0], cell.position[1], cell.position[2]);
        let (payload, added, bytes) = match options.mutation {
            Mutation::Create { id, payload } => {
                if cell.block_entity.is_some() {
                    return Err(refuse("该位置已经保存方块实体"));
                }
                let block = cell
                    .state
                    .as_ref()
                    .ok_or_else(|| refuse("缺失的区块或区段没有可确定的方块，不能补建方块实体"))?;
                let payload = crate::be_registry::creation_payload(
                    cell.version,
                    &block.name,
                    &id,
                    cell.position,
                    payload.as_ref(),
                )?;
                let bytes = payload.len();
                (Some(payload), true, bytes)
            }
            Mutation::Remove => {
                let body = cell
                    .block_entity
                    .as_ref()
                    .ok_or_else(|| refuse("该位置没有方块实体"))?;
                (None, false, body.bytes)
            }
        };
        let chunk = (position.0.div_euclid(16), position.2.div_euclid(16));
        let snapshot = self.creation.get(&chunk);
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let mut input = structure_payload_input(
            &mut reader,
            chunk,
            snapshot,
            snapshot,
            self.be_overlay.get(&chunk),
            self.be_source.get(&chunk),
        )?;
        if cell.source_identity.as_ref() != Some(&input.source_sha) {
            return Err(refuse("方块实体的原始地形来源已改变"));
        }
        let proof = crate::be_editing::structure(&input.before, position, payload.as_deref())?;
        let before = crate::sections::inspect_original_block_entity(&input.before, position)
            .map_err(|e| refuse(e.to_string()))?;
        if before.as_ref().map(|body| &body.original_compound_sha256)
            != cell.block_entity.as_ref().map(|body| &body.sha256)
        {
            return Err(refuse("方块实体载荷在准备期间已经改变"));
        }
        input.after = proof.replay(&input.before)?;
        let mut group = PreparedStructure::payload_transaction(self, budget);
        group.target_sources.insert(chunk, input.source_sha.clone());
        if let Some(snapshot) = snapshot {
            group.creation_before.insert(chunk, Some(snapshot.clone()));
            group.creation_after.insert(chunk, snapshot.clone());
            group.source_creation.insert(chunk, snapshot.clone());
        }
        if let Some(state) = cell.state {
            let original = self
                .visible_chunk_base_cached(&mut reader, chunk.0, chunk.1)?
                .0
                .ok_or_else(|| refuse("方块实体目标地形消失"))?
                .block(
                    position.0.rem_euclid(16) as u8,
                    position.1,
                    position.2.rem_euclid(16) as u8,
                )
                .cloned()
                .ok_or_else(|| refuse("方块实体目标区段消失"))?;
            group.source_blocks.push(SourceCheck {
                position,
                original,
                effective: state,
            });
            group
                .source_terrain_sha
                .insert(chunk, input.source_sha.clone());
        }
        group.overwritten_block_entities = usize::from(!added);
        group.be_chunks.push(input);
        group.be_proofs.push(proof);
        group.check_work(budget)?;
        stop(&mut cancelled)?;
        self.check_structure_cancellable(&group, false, &mut cancelled)?;
        Ok(Prepared {
            group,
            position: cell.position,
            added,
            bytes,
        })
    }
}
