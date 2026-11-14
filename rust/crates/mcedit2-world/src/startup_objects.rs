//! 原生对象行保留真实身份、根路径和载荷摘要；读取不构造密集方块快照。
use super::*;
use crate::nbt_document::Value as NbtValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub tile: bool,
    pub chunk: [i32; 2],
    pub path: String,
    pub position: [i32; 3],
    pub id: String,
    pub uuid: Option<[u8; 16]>,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub source: Source,
    pub nbt: NbtValue,
    pub data_version: Option<i32>,
    pub bytes: usize,
    #[serde(skip)]
    pub(crate) compound: Vec<u8>,
}
pub(crate) fn row(
    compound: &[u8],
    tile: bool,
    path: String,
    version: Option<i32>,
) -> Result<Row, WorldError> {
    let nbt = snapshot::value(compound)?;
    let position = if tile {
        snapshot::be_position(&nbt)?
    } else {
        let point = snapshot::entity_position(&nbt)?;
        if point
            .iter()
            .any(|value| value.floor() < f64::from(i32::MIN) || value.floor() > f64::from(i32::MAX))
        {
            return Err(refuse("对象坐标超出脚本整数范围"));
        }
        point.map(|value| value.floor() as i32)
    };
    let raw = [&[10, 0, 0][..], compound].concat();
    let root = crate::sections::parse_root(&raw).map_err(refuse)?;
    let id = root
        .require("id", 8)
        .and_then(|tag| tag.string())
        .map_err(refuse)?;
    let uuid = if !tile
        && ["UUID", "UUIDMost", "UUIDLeast"]
            .iter()
            .any(|name| root.field(name).is_some())
    {
        Some(crate::entity_editing::own_uuid(&root)?)
    } else {
        None
    };
    Ok(Row {
        source: Source {
            tile,
            chunk: [position[0].div_euclid(16), position[2].div_euclid(16)],
            path,
            position,
            id,
            uuid,
            sha256: nbt_sha(compound),
        },
        nbt,
        data_version: version,
        bytes: compound.len(),
        compound: compound.to_vec(),
    })
}
fn world_rows(
    session: &Session,
    chunk: (i32, i32),
    tile: bool,
    budget: EditWorkBudget,
) -> Result<Vec<Row>, WorldError> {
    let (raw, list) = if tile {
        let raw = session.inspect_chunk_bytes(chunk, budget)?;
        let root = crate::sections::parse_root(&raw).map_err(refuse)?;
        let list = if root.field("Level").is_some() {
            "/Level/TileEntities"
        } else {
            "/block_entities"
        };
        (raw, list)
    } else {
        let change = session.entity_input(chunk)?;
        let root = crate::sections::parse_root(&change.before).map_err(refuse)?;
        let list = if !change.source.starts_with("entity-region:") && root.field("Level").is_some()
        {
            "/Level/Entities"
        } else {
            "/Entities"
        };
        (change.before, list)
    };
    budget.check(raw.len().saturating_mul(8))?;
    let root = crate::sections::parse_root(&raw).map_err(refuse)?;
    let version = root
        .field("DataVersion")
        .map(|tag| tag.i32())
        .transpose()
        .map_err(refuse)?;
    let parent = if list.starts_with("/Level/") {
        root.require("Level", 10).map_err(refuse)?
    } else {
        &root
    };
    let name = list.rsplit('/').next().expect("实际对象列表名");
    let Some(values) = parent.field(name) else {
        return Ok(Vec::new());
    };
    if values.kind != 9 || (values.raw[0] != 10 && !(values.raw[0] == 0 && values.items.is_empty()))
    {
        return Err(refuse("对象根列表不是 Compound 列表"));
    }
    values
        .items
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let mut row = row(node.raw, tile, format!("{list}/{index}"), version)?;
            row.source.chunk = [chunk.0, chunk.1];
            Ok(row)
        })
        .collect()
}
fn matches(source: &Source, candidate: &Row) -> bool {
    if source.tile {
        candidate.source.tile
            && source.position == candidate.source.position
            && source.id == candidate.source.id
    } else if let Some(uuid) = source.uuid {
        candidate.source.uuid == Some(uuid)
    } else {
        source.chunk == candidate.source.chunk
            && source.path == candidate.source.path
            && source.sha256 == candidate.source.sha256
    }
}
fn unique(source: &Source, rows: Vec<Row>) -> Result<Option<Row>, WorldError> {
    let mut found = rows.into_iter().filter(|row| matches(source, row));
    let first = found.next();
    if found.next().is_some() {
        return Err(refuse("对象实际身份重复，须重新查询并解决来源冲突"));
    }
    Ok(first)
}
impl Working {
    pub(super) fn script_object_order(
        &mut self,
        world: usize,
        dimension: &str,
        tile: bool,
        sources: Vec<Source>,
    ) -> Result<Value, WorldError> {
        self.stop()?;
        if self.read_only(world) {
            return Err(refuse("只读来源禁止重排对象"));
        }
        let budget = self.transient_budget()?;
        let token = Arc::clone(&self.cancelled);
        let mut rows = Vec::new();
        let mut seen = BTreeSet::new();
        let mut slots = BTreeMap::<[i32; 2], Vec<usize>>::new();
        for source in sources {
            if source.tile != tile {
                return Err(refuse("对象重排种类不一致"));
            }
            let row = self
                .object_row(world, dimension, &source)?
                .ok_or_else(|| refuse("重排对象已移除"))?;
            if !seen.insert((row.source.chunk, row.source.path.clone())) {
                return Err(refuse("对象重排重复引用同一个实际根"));
            }
            let index = row
                .source
                .path
                .rsplit('/')
                .next()
                .expect("真实根路径")
                .parse::<usize>()
                .map_err(|_| refuse("对象根路径无效"))?;
            slots.entry(row.source.chunk).or_default().push(index);
            rows.push(row);
        }
        let result = if let Some(entry) = self.structures.get_mut(&world) {
            let mut work = entry.working.clone();
            let result =
                work.order_script_objects(tile, &rows, budget, || token.load(Ordering::Acquire))?;
            entry.working = work;
            result
        } else {
            let entry = self.entry(world, dimension)?;
            let mut work = entry.working.clone();
            if rows.iter().any(|row| {
                row.source.chunk
                    != [
                        row.source.position[0].div_euclid(16),
                        row.source.position[2].div_euclid(16),
                    ]
            }) {
                return Err(refuse("对象坐标与实际槽位不一致，不能重排"));
            }
            let compounds = rows.iter().map(|row| row.compound.clone()).collect();
            let (entities, bes) = if tile {
                (Vec::new(), compounds)
            } else {
                (compounds, Vec::new())
            };
            work.reorder_snapshot_objects_cancellable(entities, bes, budget, || {
                token.load(Ordering::Acquire)
            })?;
            for values in slots.values_mut() {
                values.sort_unstable();
            }
            let mut offsets = BTreeMap::<[i32; 2], usize>::new();
            let mut result = Vec::new();
            for row in &rows {
                let at = offsets.entry(row.source.chunk).or_default();
                let slot = slots[&row.source.chunk][*at];
                *at += 1;
                let prefix = row
                    .source
                    .path
                    .rsplit_once('/')
                    .expect("实际对象列表路径")
                    .0;
                let path = format!("{prefix}/{slot}");
                let actual = world_rows(&work, row.source.chunk.into(), tile, budget)?
                    .into_iter()
                    .find(|candidate| candidate.source.path == path)
                    .ok_or_else(|| refuse("重排后的实际根缺失"))?;
                if actual.compound != row.compound {
                    return Err(refuse("对象重排回执与实际载荷不一致"));
                }
                result.push(actual);
            }
            work.fold_script_history(&entry.baseline, budget)?;
            entry.working = work;
            result
        };
        self.observed.insert((world, dimension.to_owned()));
        serde_json::to_value(result).map_err(|error| refuse(error.to_string()))
    }
    pub(super) fn script_object_add(
        &mut self,
        world: usize,
        dimension: &str,
        tile: bool,
        nbt: NbtValue,
    ) -> Result<Value, WorldError> {
        self.stop()?;
        if self.read_only(world) {
            return Err(refuse("只读来源禁止新增对象"));
        }
        let budget = self.transient_budget()?;
        let token = Arc::clone(&self.cancelled);
        let result = if let Some(entry) = self.structures.get_mut(&world) {
            if entry.working.content().dimension != dimension {
                return Err(refuse("对象来源维度不一致"));
            }
            let mut work = entry.working.clone();
            let row =
                work.add_script_object(tile, &nbt, budget, || token.load(Ordering::Acquire))?;
            entry.working = work;
            row
        } else {
            let entry = self.entry(world, dimension)?;
            let mut work = entry.working.clone();
            let position = if tile {
                snapshot::be_position(&nbt)?
            } else {
                row(&nbt.payload()?, false, String::new(), None)?
                    .source
                    .position
            };
            let clip = work.capture_structure_with_budget_cancellable(
                Bounds {
                    minimum: position,
                    maximum: position,
                },
                clipboard::CaptureOptions {
                    entities: false,
                    biomes: false,
                },
                budget,
                || token.load(Ordering::Acquire),
            )?;
            let mut content = snapshot::Content::capture(&clip)?;
            if tile {
                content.block_entities = vec![snapshot::Object {
                    source_index: None,
                    nbt: nbt.clone(),
                }];
            } else {
                content.entities.push(snapshot::Object {
                    source_index: None,
                    nbt: nbt.clone(),
                });
            }
            let prepared = work.prepare_snapshot_difference_cancellable(
                &clip,
                &clip.blocks,
                &content,
                budget,
                || token.load(Ordering::Acquire),
            )?;
            let canonical = match &prepared {
                Some(prepared) => {
                    let objects = prepared
                        .object_results()
                        .ok_or_else(|| refuse("新增对象候选缺少真实写回结果"))?;
                    if tile {
                        objects.block_entities[0].clone()
                    } else {
                        objects.entities[0].clone()
                    }
                }
                None if tile => nbt.clone(), // 替换内容完全相同的方块实体没有源差异。
                None => return Err(refuse("新增实体没有形成实际写回候选")),
            };
            if let Some(prepared) = prepared {
                work.commit_snapshot_difference_cancellable(
                    prepared,
                    || token.load(Ordering::Acquire),
                    |_| {},
                )?;
            }
            let expected = row(
                &canonical.payload()?,
                tile,
                String::new(),
                clip.data_version,
            )?;
            let rows = world_rows(&work, expected.source.chunk.into(), tile, budget)?;
            let result = unique(&expected.source, rows)?
                .ok_or_else(|| refuse("新增对象未出现在实际写回来源中"))?;
            work.fold_script_history(&entry.baseline, budget)?;
            entry.working = work;
            result
        };
        self.observed.insert((world, dimension.to_owned()));
        serde_json::to_value(result).map_err(|error| refuse(error.to_string()))
    }
    pub(super) fn script_objects(
        &mut self,
        world: usize,
        dimension: &str,
        region: SelectionRegion,
        tile: bool,
    ) -> Result<Value, WorldError> {
        self.stop()?;
        let budget = self.transient_budget()?;
        let token = Arc::clone(&self.cancelled);
        let rows = if let Some(entry) = self.structures.get(&world) {
            if entry.working.content().dimension != dimension {
                return Err(refuse("对象来源维度不一致"));
            }
            entry
                .working
                .script_object_rows(region, tile, budget, || token.load(Ordering::Acquire))?
        } else {
            let region = region.resolved(
                &self
                    .inputs
                    .get(world)
                    .ok_or_else(|| refuse("对象所属世界不存在"))?
                    .world,
                dimension,
            )?;
            let entry = self.entry(world, dimension)?;
            let mut rows = Vec::new();
            let mut bytes = 0usize;
            for chunk in entry
                .working
                .terrain_chunks_cancellable(budget, || token.load(Ordering::Acquire))?
            {
                if chunk.0 < region.bounds.minimum[0].div_euclid(16)
                    || chunk.0 > region.bounds.maximum[0].div_euclid(16)
                    || chunk.1 < region.bounds.minimum[2].div_euclid(16)
                    || chunk.1 > region.bounds.maximum[2].div_euclid(16)
                {
                    continue;
                }
                for object in world_rows(&entry.working, chunk, tile, budget)? {
                    let selected = if tile {
                        region.contains(object.source.position.into())
                    } else {
                        region.contains_point(snapshot::entity_position(&object.nbt)?)
                    };
                    if !selected {
                        continue;
                    }
                    bytes = bytes
                        .checked_add(object.bytes)
                        .ok_or_else(|| refuse("对象查询工作集溢出"))?;
                    budget.check(bytes.saturating_mul(16))?;
                    rows.push(object);
                }
                if token.load(Ordering::Acquire) {
                    return Err(refuse("对象查询已取消"));
                }
            }
            rows
        };
        self.observed.insert((world, dimension.to_owned()));
        serde_json::to_value(rows).map_err(|error| refuse(error.to_string()))
    }
    fn object_row(
        &mut self,
        world: usize,
        dimension: &str,
        source: &Source,
    ) -> Result<Option<Row>, WorldError> {
        let budget = self.transient_budget()?;
        let token = Arc::clone(&self.cancelled);
        if let Some(entry) = self.structures.get(&world) {
            if entry.working.content().dimension != dimension {
                return Err(refuse("对象来源维度不一致"));
            }
            return entry
                .working
                .script_object_row_optional(source, budget, || token.load(Ordering::Acquire));
        }
        let entry = self.entry(world, dimension)?;
        let hinted = world_rows(&entry.working, source.chunk.into(), source.tile, budget)?;
        if let Some(found) = unique(source, hinted)? {
            return Ok(Some(found));
        }
        if let Some(uuid) = source.uuid.filter(|_| !source.tile) {
            let index =
                entry
                    .working
                    .scan_current_identities(&BTreeMap::new(), budget.bytes(), || {
                        token.load(Ordering::Acquire)
                    })?;
            let mut found = Vec::new();
            for chunk in index.chunks_containing(dimension, &uuid) {
                found.extend(
                    world_rows(&entry.working, chunk, false, budget)?
                        .into_iter()
                        .filter(|row| matches(source, row)),
                );
            }
            if let Some(found) = unique(source, found)? {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }
    pub(super) fn script_object(
        &mut self,
        world: usize,
        dimension: &str,
        source: Source,
    ) -> Result<Value, WorldError> {
        self.stop()?;
        let row = self.object_row(world, dimension, &source)?;
        self.observed.insert((world, dimension.to_owned()));
        serde_json::to_value(row).map_err(|error| refuse(error.to_string()))
    }
    pub(super) fn script_object_change(
        &mut self,
        world: usize,
        dimension: &str,
        source: Source,
        expected: NbtValue,
        nbt: Option<NbtValue>,
    ) -> Result<Value, WorldError> {
        self.stop()?;
        if self.read_only(world) {
            return Err(refuse("只读来源禁止修改对象"));
        }
        let before = self
            .object_row(world, dimension, &source)?
            .ok_or_else(|| refuse("对象已移除或实际身份已改变"))?;
        if before.nbt != expected {
            return Err(refuse("对象载荷已改变，不能覆盖当前来源"));
        }
        if nbt.as_ref() == Some(&expected) {
            return serde_json::to_value(before).map_err(|error| refuse(error.to_string()));
        }
        let budget = self.transient_budget()?;
        let token = Arc::clone(&self.cancelled);
        let row = if let Some(entry) = self.structures.get_mut(&world) {
            let mut work = entry.working.clone();
            let row = work.change_script_object(&before, nbt.as_ref(), budget, || {
                token.load(Ordering::Acquire)
            })?;
            entry.working = work;
            row
        } else {
            let entry = self.entry(world, dimension)?;
            let mut work = entry.working.clone();
            let row = change_world_object(&mut work, &before, nbt.as_ref(), budget, || {
                token.load(Ordering::Acquire)
            })?;
            work.fold_script_history(&entry.baseline, budget)?;
            entry.working = work;
            row
        };
        self.observed.insert((world, dimension.to_owned()));
        serde_json::to_value(row).map_err(|error| refuse(error.to_string()))
    }
}
fn change_world_object(
    work: &mut Session,
    before: &Row,
    after: Option<&NbtValue>,
    budget: EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Option<Row>, WorldError> {
    if before.source.chunk
        != [
            before.source.position[0].div_euclid(16),
            before.source.position[2].div_euclid(16),
        ]
    {
        return Err(refuse("对象坐标与实际存储槽不一致，须先修复存储归属"));
    }
    let after_be_position = if before.source.tile {
        after.map(snapshot::be_position).transpose()?
    } else {
        None
    };
    if !before.source.tile {
        let source = clipboard::OpaqueEntity {
            position: snapshot::entity_position(&before.nbt)?,
            block_position: None,
            compound: before.compound.clone(),
        };
        work.rewrite_structure_entity_cancellable(&source, after, &mut cancelled)?;
    } else if let Some(after) = after.filter(|_| after_be_position == Some(before.source.position))
    {
        let mut draft = work.inspect_object(
            ObjectTarget::BlockEntity {
                position: before.source.position.into(),
            },
            budget,
        )?;
        draft.apply_batch(&[crate::nbt_document::Operation::Replace {
            path: String::new(),
            value: after.clone(),
        }])?;
        let prepared = work.prepare_object_edit(&draft, &mut cancelled)?;
        work.commit_object_edit(prepared)?;
    } else {
        change_world_be_cell(work, before.source.position, None, budget, &mut cancelled)?;
        if let Some(after) = after {
            change_world_be_cell(
                work,
                snapshot::be_position(after)?,
                Some(after),
                budget,
                &mut cancelled,
            )?;
        }
    }
    let Some(after) = after else {
        return Ok(None);
    };
    let mut expected = row(
        &snapshot::compound(Some(&before.compound), after)?,
        before.source.tile,
        before.source.path.clone(),
        before.data_version,
    )?;
    let rows = world_rows(
        work,
        expected.source.chunk.into(),
        before.source.tile,
        budget,
    )?;
    if !expected.source.tile && expected.source.uuid.is_none() {
        let mut roots = rows
            .iter()
            .filter(|root| root.source.sha256 == expected.source.sha256);
        let root = roots
            .next()
            .ok_or_else(|| refuse("无 UUID 对象的完整写回载荷缺失"))?;
        if roots.next().is_some() {
            return Err(refuse("无 UUID 对象的写回结果重复"));
        }
        expected.source.path = root.source.path.clone();
    }
    unique(&expected.source, rows)?
        .map(Some)
        .ok_or_else(|| refuse("原生对象写回结果未出现在实际目标来源中"))
}
fn change_world_be_cell(
    work: &mut Session,
    position: [i32; 3],
    after: Option<&NbtValue>,
    budget: EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> Result<(), WorldError> {
    let snapshot = work.capture_structure_with_budget_cancellable(
        Bounds {
            minimum: position,
            maximum: position,
        },
        clipboard::CaptureOptions {
            entities: false,
            biomes: false,
        },
        budget,
        &mut cancelled,
    )?;
    let mut content = snapshot::Content::capture(&snapshot)?;
    if after.is_some() && !content.block_entities.is_empty() {
        return Err(refuse("方块实体移动目标已被占用"));
    }
    content.block_entities.clear();
    if let Some(after) = after {
        content.block_entities.push(snapshot::Object {
            source_index: None,
            nbt: after.clone(),
        });
    }
    if let Some(prepared) = work.prepare_snapshot_difference_cancellable(
        &snapshot,
        &snapshot.blocks,
        &content,
        budget,
        &mut cancelled,
    )? {
        work.commit_snapshot_difference_cancellable(prepared, &mut cancelled, |_| {})?;
    }
    Ok(())
}
